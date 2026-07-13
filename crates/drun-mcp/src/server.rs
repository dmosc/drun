//! MCP tool dispatch: implements ServerHandler to route each tool call to the
//! appropriate Session method, wrapping results as MCP CallToolResult responses.

use crate::errors::DrunError;
use crate::handler::DrunHandler;
use crate::response::{file_content, json, text};
use crate::state::{
    CheckpointSummary, SessionState, SessionSummary, SessionTreeNode, SnapshotEntry,
};
use crate::tools::{
    CheckpointReadStdstreams, DrunTools, GetSessionState, SessionBash, SessionCheckpointDrop,
    SessionCheckpointLabel, SessionCheckpointSquash, SessionClose, SessionCommit,
    SessionDeleteFile, SessionDiff, SessionExport, SessionFetch, SessionFork, SessionGetEnv,
    SessionHistory, SessionLabel, SessionMerge, SessionMount, SessionReadFile, SessionRestore,
    SessionRollback, SessionSnapshotTool, SessionWriteFile,
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
use std::{path::PathBuf, sync::Arc, time::Duration};
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
            DrunTools::CreateSession(_) => self.handle_create_session(),
            DrunTools::SessionFork(t) => self.handle_session_fork(t),
            DrunTools::SessionList(_) => self.handle_session_list(),
            DrunTools::SessionClose(t) => self.handle_session_close(t),
            DrunTools::SessionHistory(t) => self.handle_session_history(t),
            DrunTools::GetSessionState(t) => self.handle_get_session_state(t),
            DrunTools::SessionBash(t) => self.handle_session_bash(t, runtime, progress_token),
            DrunTools::SessionRollback(t) => self.handle_session_rollback(t),
            DrunTools::SessionReadFile(t) => self.handle_session_read_file(t),
            DrunTools::SessionWriteFile(t) => self.handle_session_write_file(t),
            DrunTools::SessionDeleteFile(t) => self.handle_session_delete_file(t),
            DrunTools::SessionMount(t) => self.handle_session_mount(t),
            DrunTools::SessionDiff(t) => self.handle_session_diff(t),
            DrunTools::SessionCommit(t) => self.handle_session_commit(t),
            DrunTools::SessionTree(_) => self.handle_session_tree(),
            DrunTools::ListSnapshots(_) => self.handle_list_snapshots(),
            DrunTools::SessionExport(t) => self.handle_session_export(t),
            DrunTools::SessionFetch(t) => self.handle_session_fetch(t).await,
            DrunTools::GetFetchAllowlist(_) => self.handle_get_fetch_allowlist(),
            DrunTools::SessionSnapshotTool(t) => self.handle_session_snapshot(t),
            DrunTools::SessionGetEnv(t) => self.handle_session_get_env(t),
            DrunTools::SessionRestore(t) => self.handle_session_restore(t),
            DrunTools::SessionLabel(t) => self.handle_session_label(t),
            DrunTools::SessionCheckpointLabel(t) => self.handle_session_checkpoint_label(t),
            DrunTools::SessionCheckpointSquash(t) => self.handle_session_checkpoint_squash(t),
            DrunTools::SessionMerge(t) => self.handle_session_merge(t),
            DrunTools::SessionCheckpointDrop(t) => self.handle_session_checkpoint_drop(t),
            DrunTools::CheckpointReadStdstreams(t) => self.handle_checkpoint_read_stdstreams(t),
        }
    }
}

impl DrunHandler {
    fn handle_create_session(&self) -> Result<CallToolResult, CallToolError> {
        let session_id = Uuid::new_v4().to_string();
        let session = Session::new(self.config.clone())
            .map_err(|e| DrunError::internal(e).into_tool_err())?;
        let state = SessionState::compute(&session_id, &session, None, vec![]);
        self.insert_session(session_id, session)?;
        Ok(json(&state))
    }

    fn handle_session_fork(&self, t: SessionFork) -> Result<CallToolResult, CallToolError> {
        let source_arc = self.resolve_session(&t.session_id)?;
        let forked_session = {
            let source = DrunHandler::lock_recovering(&t.session_id, &source_arc);
            let checkpoint_id = source
                .resolve_checkpoint(t.checkpoint_id, t.checkpoint_label.as_deref())
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Session::from_session(self.config.clone(), &t.session_id, &source, checkpoint_id)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?
        };
        let fork_id = Uuid::new_v4().to_string();
        let state = SessionState::compute(&fork_id, &forked_session, None, vec![]);
        self.insert_session(fork_id, forked_session)?;
        Ok(json(&state))
    }

    fn handle_session_list(&self) -> Result<CallToolResult, CallToolError> {
        let sessions = self.sessions.lock().unwrap().clone();
        Ok(json(&SessionSummary::all(&sessions)))
    }

    fn handle_session_close(&self, t: SessionClose) -> Result<CallToolResult, CallToolError> {
        let session = self
            .sessions
            .lock()
            .unwrap()
            .remove(&t.session_id)
            .ok_or_else(|| DrunError::session_not_found(&t.session_id).into_tool_err())?;
        let config = self.config.get();
        if config.snapshot_on_close {
            let output_path = config.snapshots_dir.join(format!("{}.drun", t.session_id));
            if let Some(parent_dir) = output_path.parent() {
                let _ = std::fs::create_dir_all(parent_dir);
            }
            let guard = DrunHandler::lock_recovering(&t.session_id, &session);
            let _ = guard.snapshot().write(&output_path);
        }
        Ok(text(format!("closed {}", t.session_id)))
    }

    fn handle_session_history(&self, t: SessionHistory) -> Result<CallToolResult, CallToolError> {
        self.with_session(&t.session_id, |session| {
            Ok(json(&CheckpointSummary::history(session)))
        })
    }

    fn handle_get_session_state(
        &self,
        t: GetSessionState,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session(&t.session_id, |session| {
            Ok(json(&SessionState::compute(
                &t.session_id,
                session,
                None,
                vec![],
            )))
        })
    }

    fn handle_session_bash(
        &self,
        t: SessionBash,
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
            Ok(json(&SessionState::compute(
                &t.session_id,
                session,
                Some(&previous_files),
                vec![],
            )))
        })
    }

    fn handle_session_rollback(&self, t: SessionRollback) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
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
            Ok(json(&SessionState::compute(
                &t.session_id,
                session,
                Some(&previous_files),
                vec![],
            )))
        })
    }

    fn handle_session_read_file(
        &self,
        t: SessionReadFile,
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
                .map(|l| start.saturating_add(l as usize).min(total))
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
        t: SessionWriteFile,
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
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(json(&SessionState::compute(
                &t.session_id,
                session,
                Some(&previous_files),
                vec![],
            )))
        })
    }

    fn handle_session_delete_file(
        &self,
        t: SessionDeleteFile,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
            let previous_files = session.current().files.clone();
            session
                .delete_file(&t.path)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(json(&SessionState::compute(
                &t.session_id,
                session,
                Some(&previous_files),
                vec![],
            )))
        })
    }

    fn handle_session_mount(&self, t: SessionMount) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
            let previous_files = session.current().files.clone();
            session
                .mount(std::path::Path::new(&t.path))
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(json(&SessionState::compute(
                &t.session_id,
                session,
                Some(&previous_files),
                vec![],
            )))
        })
    }

    fn handle_session_diff(&self, t: SessionDiff) -> Result<CallToolResult, CallToolError> {
        self.with_session(&t.session_id, |session| {
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
            Ok(text(if diff.is_empty() {
                "no changes".into()
            } else {
                diff
            }))
        })
    }

    fn handle_session_commit(&self, t: SessionCommit) -> Result<CallToolResult, CallToolError> {
        self.with_session(&t.session_id, |session| {
            let paths = session
                .commit(t.keys)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            let committed_files = paths
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            Ok(json(&SessionState::compute(
                &t.session_id,
                session,
                None,
                committed_files,
            )))
        })
    }

    fn handle_session_tree(&self) -> Result<CallToolResult, CallToolError> {
        let sessions = self.sessions.lock().unwrap().clone();
        Ok(json(&SessionTreeNode::forest(&sessions)))
    }

    fn handle_list_snapshots(&self) -> Result<CallToolResult, CallToolError> {
        Ok(json(&SnapshotEntry::catalog(
            &self.config.get().snapshots_dir,
        )))
    }

    fn handle_session_export(&self, t: SessionExport) -> Result<CallToolResult, CallToolError> {
        let export_root = self.config.get().export_root;
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
                if !p.starts_with(&export_root) {
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
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
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

    async fn handle_session_fetch(&self, t: SessionFetch) -> Result<CallToolResult, CallToolError> {
        self.resolve_session(&t.session_id)?;
        let config = self.config.get();
        let url_is_allowed = Self::host_from_url(&t.url).is_some_and(|h| config.domain_allowed(&h));
        if !url_is_allowed {
            return Err(DrunError::fetch_denied(&t.url).into_tool_err());
        }

        let method = t.method.as_deref().unwrap_or("GET").to_uppercase();
        let parsed_method = method.parse::<reqwest::Method>().map_err(|_| {
            DrunError::internal(format!("invalid HTTP method: {method}")).into_tool_err()
        })?;

        let builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(config.connect_timeout_ms))
            .timeout(Duration::from_millis(config.fetch_timeout_ms));
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

        let max_body = config
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
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
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
            serde_json::to_string(&self.config.get().domain_allowlist).unwrap(),
        ))
    }

    fn handle_session_snapshot(
        &self,
        t: SessionSnapshotTool,
    ) -> Result<CallToolResult, CallToolError> {
        let snapshots_dir = self.config.get().snapshots_dir;
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
                if !p.starts_with(&snapshots_dir) {
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

    fn handle_session_get_env(&self, t: SessionGetEnv) -> Result<CallToolResult, CallToolError> {
        self.resolve_session(&t.session_id)?;
        if !self.config.get().env_allowlist.contains(&t.name) {
            return Err(DrunError::env_var_denied(&t.name).into_tool_err());
        }
        let value = std::env::var(&t.name).unwrap_or_default();
        Ok(text(
            serde_json::json!({ "name": t.name, "value": value }).to_string(),
        ))
    }

    fn handle_session_restore(&self, t: SessionRestore) -> Result<CallToolResult, CallToolError> {
        let bytes = std::fs::read(&t.path).map_err(|e| DrunError::internal(e).into_tool_err())?;
        let snapshot =
            SessionSnapshot::decode(&bytes).map_err(|e| DrunError::internal(e).into_tool_err())?;
        let restored = Session::from_snapshot(self.config.clone(), snapshot)
            .map_err(|e| DrunError::internal(e).into_tool_err())?;
        let session_id = Uuid::new_v4().to_string();
        let state = SessionState::compute(&session_id, &restored, None, vec![]);
        self.insert_session(session_id, restored)?;
        Ok(json(&state))
    }

    fn handle_session_label(&self, t: SessionLabel) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
            session.set_label(t.label);
            Ok(json(&SessionState::compute(
                &t.session_id,
                session,
                None,
                vec![],
            )))
        })
    }

    fn handle_session_checkpoint_label(
        &self,
        t: SessionCheckpointLabel,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
            let checkpoint_id = t
                .checkpoint_id
                .map(|id| id as usize)
                .unwrap_or_else(|| session.current().id);
            session
                .set_checkpoint_label(checkpoint_id, t.label)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(json(&CheckpointSummary::history(session)))
        })
    }

    fn handle_session_checkpoint_squash(
        &self,
        t: SessionCheckpointSquash,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
            session
                .squash_checkpoints(
                    t.from_checkpoint_id as usize,
                    t.to_checkpoint_id as usize,
                    t.label,
                )
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(json(&CheckpointSummary::history(session)))
        })
    }

    fn handle_session_merge(&self, t: SessionMerge) -> Result<CallToolResult, CallToolError> {
        if t.session_id == t.source_session_id {
            return Err(DrunError::internal("cannot merge a session with itself").into_tool_err());
        }
        let source_arc = self.resolve_session(&t.source_session_id)?;
        let source = DrunHandler::lock_recovering(&t.source_session_id, &source_arc);
        let source_checkpoint_id = source
            .resolve_checkpoint(t.source_checkpoint_id, t.source_checkpoint_label.as_deref())
            .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
        self.with_session_mut(&t.session_id, |session| {
            session
                .merge_from(&source, source_checkpoint_id, t.keys)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(json(&SessionState::compute(
                &t.session_id,
                session,
                None,
                vec![],
            )))
        })
    }

    fn handle_session_checkpoint_drop(
        &self,
        t: SessionCheckpointDrop,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session_mut(&t.session_id, |session| {
            session
                .drop_checkpoints(t.from_checkpoint_id as usize, t.to_checkpoint_id as usize)
                .map_err(|e| DrunError::from_exec(e).into_tool_err())?;
            Ok(json(&CheckpointSummary::history(session)))
        })
    }

    fn handle_checkpoint_read_stdstreams(
        &self,
        t: CheckpointReadStdstreams,
    ) -> Result<CallToolResult, CallToolError> {
        self.with_session(&t.session_id, |session| {
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
    use crate::tools::HttpHeader;
    use drun_core::Config;
    use rust_mcp_sdk::schema::ContentBlock;
    use std::sync::Mutex;

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

    fn insert_session(handler: &DrunHandler, id: &str) {
        handler.sessions.lock().unwrap().insert(
            id.to_string(),
            Arc::new(Mutex::new(Session::new(handler.config.clone()).unwrap())),
        );
    }

    fn result_text(result: &CallToolResult) -> &str {
        match &result.content[0] {
            ContentBlock::TextContent(tc) => &tc.text,
            _ => panic!("expected text content"),
        }
    }

    fn result_json(result: &CallToolResult) -> serde_json::Value {
        serde_json::from_str(result_text(result)).unwrap()
    }

    #[test]
    fn create_session_succeeds_and_registers_the_session() {
        let handler = DrunHandler::new(Config::default());
        let result = handler.handle_create_session().unwrap();
        assert_eq!(handler.sessions.lock().unwrap().len(), 1);
        assert!(result_text(&result).contains("checkpoint_id"));
    }

    #[test]
    fn create_session_rejects_once_max_sessions_is_reached() {
        let config = Config {
            max_sessions: Some(1),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        handler.handle_create_session().unwrap();
        let err = handler.handle_create_session().unwrap_err();
        assert!(err.to_string().contains("session_limit_reached"));
    }

    #[test]
    fn session_fork_rejects_once_max_sessions_is_reached() {
        let config = Config {
            max_sessions: Some(1),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_session(&handler, "source");

        let err = handler
            .handle_session_fork(SessionFork {
                session_id: "source".to_string(),
                checkpoint_id: None,
                checkpoint_label: None,
            })
            .unwrap_err();

        assert!(err.to_string().contains("session_limit_reached"));
        assert_eq!(handler.sessions.lock().unwrap().len(), 1);
    }

    #[test]
    fn session_restore_rejects_once_max_sessions_is_reached() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            max_sessions: Some(1),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_session(&handler, "original");
        let snapshot_path = dir.path().join("original.drun");
        {
            let sessions = handler.sessions.lock().unwrap();
            let session = sessions.get("original").unwrap().lock().unwrap();
            session.snapshot().write(&snapshot_path).unwrap();
        }

        let err = handler
            .handle_session_restore(SessionRestore {
                path: snapshot_path.to_string_lossy().into_owned(),
            })
            .unwrap_err();

        assert!(err.to_string().contains("session_limit_reached"));
        assert_eq!(handler.sessions.lock().unwrap().len(), 1);
    }

    #[test]
    fn session_fork_returns_session_not_found_for_missing_source() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_fork(SessionFork {
                session_id: "missing".to_string(),
                checkpoint_id: None,
                checkpoint_label: None,
            })
            .unwrap_err();
        assert!(err.to_string().contains("session_not_found"));
    }

    #[test]
    fn session_fork_inherits_files_from_the_source_checkpoint() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "source");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("source")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"hi".to_vec())
                .unwrap();
        }

        let result = handler
            .handle_session_fork(SessionFork {
                session_id: "source".to_string(),
                checkpoint_id: None,
                checkpoint_label: None,
            })
            .unwrap();
        assert_eq!(handler.sessions.lock().unwrap().len(), 2);
        let json = result_json(&result);
        assert_eq!(json["workspace_file_count"], 1);
    }

    #[test]
    fn session_fork_recovers_from_a_poisoned_source_lock_instead_of_panicking() {
        let handler = DrunHandler::new(Config::default());
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
            .handle_session_fork(SessionFork {
                session_id: "source".to_string(),
                checkpoint_id: None,
                checkpoint_label: None,
            })
            .unwrap();
        assert!(result_text(&result).contains("checkpoint_id"));
    }

    #[test]
    fn session_close_removes_the_session_from_the_map() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        handler
            .handle_session_close(SessionClose {
                session_id: "s1".to_string(),
            })
            .unwrap();
        assert!(!handler.sessions.lock().unwrap().contains_key("s1"));
    }

    #[test]
    fn session_close_returns_session_not_found_for_missing_id() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_close(SessionClose {
                session_id: "missing".to_string(),
            })
            .unwrap_err();
        assert!(err.to_string().contains("session_not_found"));
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
        insert_session(&handler, "s1");

        handler
            .handle_session_close(SessionClose {
                session_id: "s1".to_string(),
            })
            .unwrap();

        assert!(dir.path().join("s1.drun").exists());
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
        insert_session(&handler, "s1");
        let session_arc = handler.sessions.lock().unwrap().get("s1").unwrap().clone();
        let arc_for_panic = session_arc.clone();
        let _ = std::thread::spawn(move || {
            let _guard = arc_for_panic.lock().unwrap();
            panic!("simulated panic while holding the session lock");
        })
        .join();
        assert!(session_arc.is_poisoned());

        handler
            .handle_session_close(SessionClose {
                session_id: "s1".to_string(),
            })
            .unwrap();

        assert!(dir.path().join("s1.drun").exists());
    }

    #[test]
    fn session_rollback_requires_a_checkpoint_id_or_label() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let err = handler
            .handle_session_rollback(SessionRollback {
                session_id: "s1".to_string(),
                checkpoint_id: None,
                checkpoint_label: None,
            })
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("provide checkpoint_id or checkpoint_label")
        );
    }

    #[test]
    fn session_rollback_moves_the_head_to_the_given_checkpoint() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"1".to_vec())
                .unwrap();
        }

        let result = handler
            .handle_session_rollback(SessionRollback {
                session_id: "s1".to_string(),
                checkpoint_id: Some(0),
                checkpoint_label: None,
            })
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["checkpoint_id"], 0);
    }

    #[test]
    fn session_rollback_returns_checkpoint_not_found_for_an_unknown_checkpoint_id() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let err = handler
            .handle_session_rollback(SessionRollback {
                session_id: "s1".to_string(),
                checkpoint_id: Some(99),
                checkpoint_label: None,
            })
            .unwrap_err();
        assert!(err.to_string().contains("checkpoint_not_found"));
    }

    #[test]
    fn session_read_file_returns_full_utf8_content_without_offset_or_limit() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"hello world".to_vec())
                .unwrap();
        }

        let result = handler
            .handle_session_read_file(SessionReadFile {
                session_id: "s1".to_string(),
                path: "a.txt".to_string(),
                offset: None,
                limit: None,
            })
            .unwrap();
        assert_eq!(result_text(&result), "hello world");
    }

    #[test]
    fn session_read_file_pages_through_content_with_offset_and_limit() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"hello world".to_vec())
                .unwrap();
        }

        let result = handler
            .handle_session_read_file(SessionReadFile {
                session_id: "s1".to_string(),
                path: "a.txt".to_string(),
                offset: Some(6),
                limit: Some(5),
            })
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["content"], "world");
        assert_eq!(json["has_more"], false);
        assert_eq!(json["total_bytes"], 11);
    }

    #[test]
    fn session_read_file_clamps_a_limit_that_would_overflow_past_total_bytes() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"hello world".to_vec())
                .unwrap();
        }

        let result = handler
            .handle_session_read_file(SessionReadFile {
                session_id: "s1".to_string(),
                path: "a.txt".to_string(),
                offset: Some(6),
                limit: Some(u64::MAX),
            })
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["content"], "world");
        assert_eq!(json["has_more"], false);
    }

    #[test]
    fn session_read_file_base64_encodes_non_utf8_content_when_paginated() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let bytes = vec![0xff, 0xfe, 0xfd];
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("bin.dat", bytes)
                .unwrap();
        }

        let result = handler
            .handle_session_read_file(SessionReadFile {
                session_id: "s1".to_string(),
                path: "bin.dat".to_string(),
                offset: Some(0),
                limit: Some(3),
            })
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["encoding"], "base64");
    }

    #[test]
    fn session_read_file_returns_file_not_found_for_missing_path() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let err = handler
            .handle_session_read_file(SessionReadFile {
                session_id: "s1".to_string(),
                path: "missing.txt".to_string(),
                offset: None,
                limit: None,
            })
            .unwrap_err();
        assert!(err.to_string().contains("file_not_found"));
    }

    #[test]
    fn session_write_file_decodes_base64_content() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let encoded = BASE64.encode(b"hello");

        handler
            .handle_session_write_file(SessionWriteFile {
                session_id: "s1".to_string(),
                path: "a.txt".to_string(),
                content: encoded,
                is_base64: Some(true),
            })
            .unwrap();

        let sessions = handler.sessions.lock().unwrap();
        let session = sessions.get("s1").unwrap().lock().unwrap();
        assert_eq!(session.current().files["a.txt"].as_slice(), b"hello");
    }

    #[test]
    fn session_write_file_rejects_invalid_base64() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let err = handler
            .handle_session_write_file(SessionWriteFile {
                session_id: "s1".to_string(),
                path: "a.txt".to_string(),
                content: "not valid base64!!".to_string(),
                is_base64: Some(true),
            })
            .unwrap_err();
        assert!(err.to_string().contains("base64 decode error"));
    }

    #[test]
    fn session_write_file_returns_invalid_workspace_path_for_a_path_escaping_the_workspace() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let err = handler
            .handle_session_write_file(SessionWriteFile {
                session_id: "s1".to_string(),
                path: "../escape.txt".to_string(),
                content: "hi".to_string(),
                is_base64: Some(false),
            })
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
        insert_session(&handler, "s1");
        let err = handler
            .handle_session_write_file(SessionWriteFile {
                session_id: "s1".to_string(),
                path: "a.txt".to_string(),
                content: "hi".to_string(),
                is_base64: Some(false),
            })
            .unwrap_err();
        assert!(err.to_string().contains("workspace_size_exceeded"));
    }

    #[test]
    fn session_diff_defaults_from_checkpoint_zero_to_the_current_checkpoint() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"hi".to_vec())
                .unwrap();
        }

        let result = handler
            .handle_session_diff(SessionDiff {
                session_id: "s1".to_string(),
                from_checkpoint_id: None,
                from_checkpoint_label: None,
                to_checkpoint_id: None,
                to_checkpoint_label: None,
            })
            .unwrap();
        assert!(result_text(&result).contains("a.txt"));
    }

    #[test]
    fn session_diff_reports_no_changes_between_identical_checkpoints() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");

        let result = handler
            .handle_session_diff(SessionDiff {
                session_id: "s1".to_string(),
                from_checkpoint_id: Some(0),
                from_checkpoint_label: None,
                to_checkpoint_id: Some(0),
                to_checkpoint_label: None,
            })
            .unwrap();
        assert_eq!(result_text(&result), "no changes");
    }

    #[test]
    fn session_export_rejects_a_path_containing_dotdot() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_export(SessionExport {
                session_id: "s1".to_string(),
                output_dir: Some("../escape".to_string()),
                keys: None,
            })
            .unwrap_err();
        assert!(err.to_string().contains("export_denied"));
    }

    #[test]
    fn session_export_rejects_a_directory_outside_the_export_root() {
        let config = Config {
            export_root: PathBuf::from("drun-export"),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        let err = handler
            .handle_session_export(SessionExport {
                session_id: "s1".to_string(),
                output_dir: Some("/tmp/somewhere-else".to_string()),
                keys: None,
            })
            .unwrap_err();
        assert!(err.to_string().contains("export_denied"));
    }

    #[test]
    fn session_export_writes_files_under_the_export_root() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            export_root: dir.path().to_path_buf(),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("out.txt", b"data".to_vec())
                .unwrap();
        }

        handler
            .handle_session_export(SessionExport {
                session_id: "s1".to_string(),
                output_dir: Some(dir.path().join("sub").to_string_lossy().into_owned()),
                keys: Some(vec!["out.txt".to_string()]),
            })
            .unwrap();
        assert!(dir.path().join("sub/out.txt").exists());
    }

    #[test]
    fn session_snapshot_rejects_a_path_containing_dotdot() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_snapshot(SessionSnapshotTool {
                session_id: "s1".to_string(),
                path: Some("../escape.drun".to_string()),
            })
            .unwrap_err();
        assert!(err.to_string().contains("snapshot_denied"));
    }

    #[test]
    fn session_snapshot_writes_under_the_default_snapshots_dir() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            snapshots_dir: dir.path().to_path_buf(),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_session(&handler, "s1");

        handler
            .handle_session_snapshot(SessionSnapshotTool {
                session_id: "s1".to_string(),
                path: None,
            })
            .unwrap();

        assert!(dir.path().join("s1.drun").exists());
    }

    #[test]
    fn session_get_env_returns_session_not_found_for_missing_session() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_get_env(SessionGetEnv {
                session_id: "missing".to_string(),
                name: "PATH".to_string(),
            })
            .unwrap_err();
        assert!(err.to_string().contains("session_not_found"));
    }

    #[test]
    fn session_get_env_rejects_names_outside_the_allowlist() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let err = handler
            .handle_session_get_env(SessionGetEnv {
                session_id: "s1".to_string(),
                name: "SECRET".to_string(),
            })
            .unwrap_err();
        assert!(err.to_string().contains("env_var_denied"));
    }

    #[test]
    fn session_get_env_returns_empty_string_for_an_unset_allowlisted_variable() {
        let config = Config {
            env_allowlist: vec!["DRUN_TEST_VAR_NOT_SET".to_string()],
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_session(&handler, "s1");

        let result = handler
            .handle_session_get_env(SessionGetEnv {
                session_id: "s1".to_string(),
                name: "DRUN_TEST_VAR_NOT_SET".to_string(),
            })
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["value"], "");
    }

    #[test]
    fn session_checkpoint_label_defaults_to_the_current_checkpoint() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("a.txt", b"hi".to_vec())
                .unwrap();
        }

        let result = handler
            .handle_session_checkpoint_label(SessionCheckpointLabel {
                session_id: "s1".to_string(),
                checkpoint_id: None,
                label: "milestone".to_string(),
            })
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
    fn session_merge_rejects_merging_a_session_with_itself() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_merge(SessionMerge {
                session_id: "s1".to_string(),
                source_session_id: "s1".to_string(),
                source_checkpoint_id: None,
                source_checkpoint_label: None,
                keys: None,
            })
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("cannot merge a session with itself")
        );
    }

    #[test]
    fn session_merge_returns_session_not_found_for_missing_source() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "target");
        let err = handler
            .handle_session_merge(SessionMerge {
                session_id: "target".to_string(),
                source_session_id: "missing-source".to_string(),
                source_checkpoint_id: None,
                source_checkpoint_label: None,
                keys: None,
            })
            .unwrap_err();
        assert!(err.to_string().contains("session_not_found"));
    }

    #[test]
    fn session_merge_overlays_files_from_the_source_session() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "target");
        insert_session(&handler, "source");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("source")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("shared.txt", b"from source".to_vec())
                .unwrap();
        }

        let result = handler
            .handle_session_merge(SessionMerge {
                session_id: "target".to_string(),
                source_session_id: "source".to_string(),
                source_checkpoint_id: None,
                source_checkpoint_label: None,
                keys: None,
            })
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["workspace_file_count"], 1);
    }

    #[test]
    fn session_merge_recovers_from_a_poisoned_source_lock_instead_of_panicking() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "target");
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
            .handle_session_merge(SessionMerge {
                session_id: "target".to_string(),
                source_session_id: "source".to_string(),
                source_checkpoint_id: None,
                source_checkpoint_label: None,
                keys: None,
            })
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["checkpoint_id"], 1);
    }

    #[test]
    fn checkpoint_read_stdstreams_rejects_an_unknown_stream_name() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let err = handler
            .handle_checkpoint_read_stdstreams(CheckpointReadStdstreams {
                session_id: "s1".to_string(),
                checkpoint_id: None,
                stream: Some("stdxyz".to_string()),
                offset: None,
                limit: None,
            })
            .unwrap_err();
        assert!(err.to_string().contains("unknown stream"));
    }

    #[test]
    fn checkpoint_read_stdstreams_defaults_to_stdout_of_the_current_checkpoint() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let result = handler
            .handle_checkpoint_read_stdstreams(CheckpointReadStdstreams {
                session_id: "s1".to_string(),
                checkpoint_id: None,
                stream: None,
                offset: None,
                limit: None,
            })
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["stream"], "stdout");
        assert_eq!(json["checkpoint_id"], 0);
        assert_eq!(json["total_bytes"], 0);
    }

    #[test]
    fn checkpoint_read_stdstreams_returns_checkpoint_does_not_exist_for_bad_id() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let err = handler
            .handle_checkpoint_read_stdstreams(CheckpointReadStdstreams {
                session_id: "s1".to_string(),
                checkpoint_id: Some(99),
                stream: None,
                offset: None,
                limit: None,
            })
            .unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn checkpoint_read_stdstreams_clamps_a_limit_that_would_overflow_past_total_bytes() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");

        let result = handler
            .handle_checkpoint_read_stdstreams(CheckpointReadStdstreams {
                session_id: "s1".to_string(),
                checkpoint_id: Some(0),
                stream: None,
                offset: Some(0),
                limit: Some(u64::MAX),
            })
            .unwrap();
        let json = result_json(&result);
        assert_eq!(json["content"], "");
    }

    #[tokio::test]
    async fn session_fetch_returns_session_not_found_for_missing_session() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_fetch(SessionFetch {
                session_id: "missing".to_string(),
                url: "https://pypi.org/simple/".to_string(),
                method: None,
                headers: None,
                body: None,
                save_to: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("session_not_found"));
    }

    #[tokio::test]
    async fn session_fetch_denies_urls_outside_the_domain_allowlist() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let err = handler
            .handle_session_fetch(SessionFetch {
                session_id: "s1".to_string(),
                url: "https://evil.example.com/data".to_string(),
                method: None,
                headers: None,
                body: None,
                save_to: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("fetch_denied"));
    }

    fn fetch_test_config(mock_uri: &str) -> Config {
        Config {
            domain_allowlist: vec![DrunHandler::host_from_url(mock_uri).unwrap()],
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn session_fetch_saves_the_response_body_under_the_default_download_path() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/data.json"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(r#"{"ok":true}"#, "application/json"),
            )
            .mount(&mock_server)
            .await;

        let handler = DrunHandler::new(fetch_test_config(&mock_server.uri()));
        insert_session(&handler, "s1");
        let result = handler
            .handle_session_fetch(SessionFetch {
                session_id: "s1".to_string(),
                url: format!("{}/data.json", mock_server.uri()),
                method: None,
                headers: None,
                body: None,
                save_to: None,
            })
            .await
            .unwrap();

        let json = result_json(&result);
        assert_eq!(json["status"], 200);
        assert_eq!(json["content_type"], "application/json");
        assert_eq!(json["saved_to"], "downloads/data.json");

        let sessions = handler.sessions.lock().unwrap();
        let session = sessions.get("s1").unwrap().lock().unwrap();
        let saved = session.current().files.get("downloads/data.json").unwrap();
        assert_eq!(saved.as_slice(), br#"{"ok":true}"#);
    }

    #[tokio::test]
    async fn session_fetch_honors_an_explicit_save_to_path_and_method() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/submit"))
            .respond_with(ResponseTemplate::new(201).set_body_string("created"))
            .mount(&mock_server)
            .await;

        let handler = DrunHandler::new(fetch_test_config(&mock_server.uri()));
        insert_session(&handler, "s1");
        let result = handler
            .handle_session_fetch(SessionFetch {
                session_id: "s1".to_string(),
                url: format!("{}/submit", mock_server.uri()),
                method: Some("post".to_string()),
                headers: None,
                body: Some("payload".to_string()),
                save_to: Some("out/response.txt".to_string()),
            })
            .await
            .unwrap();

        let json = result_json(&result);
        assert_eq!(json["status"], 201);
        assert_eq!(json["saved_to"], "out/response.txt");

        let sessions = handler.sessions.lock().unwrap();
        let session = sessions.get("s1").unwrap().lock().unwrap();
        assert_eq!(
            session
                .current()
                .files
                .get("out/response.txt")
                .unwrap()
                .as_slice(),
            b"created"
        );
    }

    #[tokio::test]
    async fn session_fetch_rejects_an_invalid_http_method() {
        use wiremock::MockServer;

        // No Mock is registered: an invalid method token must be rejected
        // before any request reaches the (local, offline) mock server.
        let mock_server = MockServer::start().await;
        let handler = DrunHandler::new(fetch_test_config(&mock_server.uri()));
        insert_session(&handler, "s1");
        let err = handler
            .handle_session_fetch(SessionFetch {
                session_id: "s1".to_string(),
                url: mock_server.uri(),
                method: Some("IN VALID".to_string()),
                headers: None,
                body: None,
                save_to: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid HTTP method"));
    }

    #[tokio::test]
    async fn session_fetch_rejects_a_response_body_over_the_configured_limit() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 2048]))
            .mount(&mock_server)
            .await;

        let mut config = fetch_test_config(&mock_server.uri());
        config.max_workspace_mb = Some(0);
        let handler = DrunHandler::new(config);
        insert_session(&handler, "s1");
        let err = handler
            .handle_session_fetch(SessionFetch {
                session_id: "s1".to_string(),
                url: mock_server.uri(),
                method: None,
                headers: None,
                body: None,
                save_to: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("exceeds"));
    }

    #[tokio::test]
    async fn session_fetch_forwards_custom_headers_to_the_request() {
        use wiremock::matchers::{header, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("x-api-key", "secret"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let handler = DrunHandler::new(fetch_test_config(&mock_server.uri()));
        insert_session(&handler, "s1");
        let result = handler
            .handle_session_fetch(SessionFetch {
                session_id: "s1".to_string(),
                url: mock_server.uri(),
                method: None,
                headers: Some(vec![HttpHeader {
                    name: "x-api-key".to_string(),
                    value: "secret".to_string(),
                }]),
                body: None,
                save_to: None,
            })
            .await
            .unwrap();
        assert_eq!(result_json(&result)["status"], 200);
    }

    #[test]
    fn session_list_reports_every_active_session() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        insert_session(&handler, "s2");
        let result = handler.handle_session_list().unwrap();
        let json = result_json(&result);
        assert_eq!(json.as_array().unwrap().len(), 2);
    }

    #[test]
    fn session_history_returns_the_checkpoint_list_for_a_session() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let result = handler
            .handle_session_history(SessionHistory {
                session_id: "s1".to_string(),
            })
            .unwrap();
        assert!(result_text(&result).contains("checkpoint_id"));
    }

    #[test]
    fn get_session_state_returns_session_not_found_for_missing_session() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_get_session_state(GetSessionState {
                session_id: "missing".to_string(),
            })
            .unwrap_err();
        assert!(err.to_string().contains("session_not_found"));
    }

    #[test]
    fn get_session_state_reports_the_current_checkpoint() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let result = handler
            .handle_get_session_state(GetSessionState {
                session_id: "s1".to_string(),
            })
            .unwrap();
        assert_eq!(result_json(&result)["checkpoint_id"], 0);
    }

    #[test]
    fn session_delete_file_removes_the_file_and_creates_a_checkpoint() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            let mut session = sessions.get("s1").unwrap().lock().unwrap();
            session.write_file("a.txt", b"hi".to_vec()).unwrap();
        }

        let result = handler
            .handle_session_delete_file(SessionDeleteFile {
                session_id: "s1".to_string(),
                path: "a.txt".to_string(),
            })
            .unwrap();
        assert_eq!(result_json(&result)["workspace_file_count"], 0);
    }

    #[test]
    fn session_delete_file_returns_file_not_found_for_a_missing_path() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let err = handler
            .handle_session_delete_file(SessionDeleteFile {
                session_id: "s1".to_string(),
                path: "missing.txt".to_string(),
            })
            .unwrap_err();
        assert!(err.to_string().contains("file_not_found"));
    }

    #[test]
    fn session_mount_loads_a_host_directory_into_the_workspace() {
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("a.txt"), b"hi").unwrap();

        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let result = handler
            .handle_session_mount(SessionMount {
                session_id: "s1".to_string(),
                path: source.path().to_string_lossy().into_owned(),
            })
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
        insert_session(&handler, "s1");
        let err = handler
            .handle_session_mount(SessionMount {
                session_id: "s1".to_string(),
                path: source.path().to_string_lossy().into_owned(),
            })
            .unwrap_err();
        assert!(err.to_string().contains("mount_denied"));
    }

    #[test]
    fn session_commit_writes_back_a_changed_mounted_file_to_the_host() {
        let source = tempfile::tempdir().unwrap();
        let host_file = source.path().join("a.txt");
        std::fs::write(&host_file, b"original").unwrap();

        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            let mut session = sessions.get("s1").unwrap().lock().unwrap();
            session.mount(&host_file).unwrap();
            session.write_file("a.txt", b"changed".to_vec()).unwrap();
        }

        let result = handler
            .handle_session_commit(SessionCommit {
                session_id: "s1".to_string(),
                keys: None,
            })
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
    fn session_tree_reflects_the_current_sessions() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let result = handler.handle_session_tree().unwrap();
        assert!(result_text(&result).contains("s1"));
    }

    #[test]
    fn list_snapshots_returns_an_empty_catalog_for_a_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            snapshots_dir: dir.path().join("does-not-exist"),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        let result = handler.handle_list_snapshots().unwrap();
        assert_eq!(result_json(&result), serde_json::json!([]));
    }

    #[test]
    fn get_fetch_allowlist_returns_the_configured_domains() {
        let config = Config {
            domain_allowlist: vec!["pypi.org".to_string()],
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        let result = handler.handle_get_fetch_allowlist().unwrap();
        assert_eq!(result_json(&result), serde_json::json!(["pypi.org"]));
    }

    #[test]
    fn session_snapshot_writes_to_an_explicit_path_under_snapshots_dir() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            snapshots_dir: dir.path().to_path_buf(),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_session(&handler, "s1");

        handler
            .handle_session_snapshot(SessionSnapshotTool {
                session_id: "s1".to_string(),
                path: Some(
                    dir.path()
                        .join("custom.drun")
                        .to_string_lossy()
                        .into_owned(),
                ),
            })
            .unwrap();
        assert!(dir.path().join("custom.drun").exists());
    }

    #[test]
    fn session_export_defaults_to_export_root_slash_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            export_root: dir.path().to_path_buf(),
            ..Config::default()
        };
        let handler = DrunHandler::new(config);
        insert_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            sessions
                .get("s1")
                .unwrap()
                .lock()
                .unwrap()
                .write_file("out.txt", b"data".to_vec())
                .unwrap();
        }

        handler
            .handle_session_export(SessionExport {
                session_id: "s1".to_string(),
                output_dir: None,
                keys: Some(vec!["out.txt".to_string()]),
            })
            .unwrap();
        assert!(dir.path().join("s1/out.txt").exists());
    }

    #[test]
    fn session_restore_recreates_a_session_from_a_snapshot_file() {
        let dir = tempfile::tempdir().unwrap();
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "original");
        {
            let sessions = handler.sessions.lock().unwrap();
            let mut session = sessions.get("original").unwrap().lock().unwrap();
            session.write_file("a.txt", b"hi".to_vec()).unwrap();
        }
        let snapshot_path = dir.path().join("original.drun");
        {
            let sessions = handler.sessions.lock().unwrap();
            let session = sessions.get("original").unwrap().lock().unwrap();
            session.snapshot().write(&snapshot_path).unwrap();
        }

        let result = handler
            .handle_session_restore(SessionRestore {
                path: snapshot_path.to_string_lossy().into_owned(),
            })
            .unwrap();
        assert_eq!(result_json(&result)["workspace_file_count"], 1);
        assert_eq!(handler.sessions.lock().unwrap().len(), 2);
    }

    #[test]
    fn session_restore_returns_an_error_for_a_missing_file() {
        let handler = DrunHandler::new(Config::default());
        let err = handler
            .handle_session_restore(SessionRestore {
                path: "/nonexistent/path.drun".to_string(),
            })
            .unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn session_label_sets_the_session_label() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        let result = handler
            .handle_session_label(SessionLabel {
                session_id: "s1".to_string(),
                label: "milestone".to_string(),
            })
            .unwrap();
        let sessions = handler.sessions.lock().unwrap();
        let session = sessions.get("s1").unwrap().lock().unwrap();
        assert_eq!(session.label.as_deref(), Some("milestone"));
        drop(session);
        let _ = result;
    }

    #[test]
    fn session_checkpoint_squash_merges_a_checkpoint_range() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            let mut session = sessions.get("s1").unwrap().lock().unwrap();
            session.write_file("a.txt", b"one".to_vec()).unwrap();
            session.write_file("a.txt", b"two".to_vec()).unwrap();
        }

        let result = handler
            .handle_session_checkpoint_squash(SessionCheckpointSquash {
                session_id: "s1".to_string(),
                from_checkpoint_id: 1,
                to_checkpoint_id: 2,
                label: Some("squashed".to_string()),
            })
            .unwrap();
        assert!(result_text(&result).contains("squashed"));
    }

    #[test]
    fn session_checkpoint_drop_removes_a_checkpoint_range() {
        let handler = DrunHandler::new(Config::default());
        insert_session(&handler, "s1");
        {
            let sessions = handler.sessions.lock().unwrap();
            let mut session = sessions.get("s1").unwrap().lock().unwrap();
            session.write_file("a.txt", b"one".to_vec()).unwrap();
            session.write_file("a.txt", b"two".to_vec()).unwrap();
        }

        let result = handler
            .handle_session_checkpoint_drop(SessionCheckpointDrop {
                session_id: "s1".to_string(),
                from_checkpoint_id: 1,
                to_checkpoint_id: 1,
            })
            .unwrap();
        let sessions = handler.sessions.lock().unwrap();
        let session = sessions.get("s1").unwrap().lock().unwrap();
        assert_eq!(session.history().len(), 2);
        let _ = result;
    }
}

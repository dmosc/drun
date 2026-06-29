//! MCP tool dispatch: implements ServerHandler to route each tool call to the
//! appropriate Session method, wrapping results as MCP CallToolResult responses.

use crate::errors::DrunError;
use crate::handler::DrunHandler;
use crate::response::{file_content, text};
use crate::state::{
    build_checkpoint_history, build_session_list, build_session_state, build_session_tree,
    build_snapshot_catalog,
};
use crate::tools::DrunTools;
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
            DrunTools::CreateSessionTool(_) => {
                if let Some(max) = self.config.max_sessions {
                    if self.sessions.lock().unwrap().len() >= max {
                        return Err(DrunError::session_limit_reached(max).into_tool_err());
                    }
                }
                let session_id = Uuid::new_v4().to_string();
                let session = Session::new(&self.config)
                    .map_err(|e| DrunError::internal(e).into_tool_err())?;
                let state = build_session_state(&session_id, &session, None, vec![]);
                self.sessions
                    .lock()
                    .unwrap()
                    .insert(session_id, Arc::new(Mutex::new(session)));
                Ok(text(state))
            }

            DrunTools::SessionForkTool(t) => {
                let source_arc = self
                    .sessions
                    .lock()
                    .unwrap()
                    .get(&t.session_id)
                    .ok_or_else(|| DrunError::session_not_found(&t.session_id).into_tool_err())?
                    .clone();
                let forked_session = {
                    let source = source_arc.lock().unwrap();
                    let checkpoint_id = match (t.checkpoint_id, t.checkpoint_label.as_deref()) {
                        (_, Some(lbl)) => {
                            Some(source.checkpoint_by_label(lbl).ok_or_else(|| {
                                DrunError::internal(format!("no checkpoint with label '{lbl}'"))
                                    .into_tool_err()
                            })?)
                        }
                        (Some(id), None) => Some(id as usize),
                        (None, None) => None,
                    };
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

            DrunTools::SessionListTool(_) => {
                let sessions = self.sessions.lock().unwrap().clone();
                Ok(text(build_session_list(&sessions)))
            }

            DrunTools::SessionCloseTool(t) => {
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

            DrunTools::SessionHistoryTool(t) => self.with_session(&t.session_id, |session| {
                Ok(text(build_checkpoint_history(session)))
            }),

            DrunTools::GetSessionStateTool(t) => self.with_session(&t.session_id, |session| {
                Ok(text(build_session_state(
                    &t.session_id,
                    session,
                    None,
                    vec![],
                )))
            }),

            DrunTools::SessionBashTool(t) => {
                let progress_tx = spawn_progress_forwarder(runtime.clone(), progress_token.clone());
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

            DrunTools::SessionRollbackTool(t) => self.with_session_mut(&t.session_id, |session| {
                let checkpoint_id = match (t.checkpoint_id, t.checkpoint_label.as_deref()) {
                    (_, Some(lbl)) => session.checkpoint_by_label(lbl).ok_or_else(|| {
                        DrunError::internal(format!("no checkpoint with label '{lbl}'"))
                            .into_tool_err()
                    })?,
                    (Some(id), None) => id as usize,
                    (None, None) => {
                        return Err(DrunError::internal(
                            "provide checkpoint_id or checkpoint_label",
                        )
                        .into_tool_err());
                    }
                };
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
            }),

            DrunTools::SessionReadFileTool(t) => self.with_session(&t.session_id, |session| {
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
            }),

            DrunTools::SessionWriteFileTool(t) => self.with_session_mut(&t.session_id, |session| {
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
            }),

            DrunTools::SessionDeleteFileTool(t) => {
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

            DrunTools::SessionMountTool(t) => self.with_session_mut(&t.session_id, |session| {
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
            }),

            DrunTools::SessionDiffTool(t) => self.with_session(&t.session_id, |session| {
                let from = match (t.from_checkpoint_id, t.from_checkpoint_label.as_deref()) {
                    (_, Some(lbl)) => session.checkpoint_by_label(lbl).ok_or_else(|| {
                        DrunError::internal(format!("no checkpoint with label '{lbl}'"))
                            .into_tool_err()
                    })?,
                    (Some(id), None) => id as usize,
                    (None, None) => 0,
                };
                let to = match (t.to_checkpoint_id, t.to_checkpoint_label.as_deref()) {
                    (_, Some(lbl)) => session.checkpoint_by_label(lbl).ok_or_else(|| {
                        DrunError::internal(format!("no checkpoint with label '{lbl}'"))
                            .into_tool_err()
                    })?,
                    (Some(id), None) => id as usize,
                    (None, None) => session.current().id,
                };
                let diff = session
                    .diff(from, to)
                    .map_err(|e| DrunError::internal(e).into_tool_err())?;
                Ok(text(if diff.is_empty() {
                    "no changes".into()
                } else {
                    diff
                }))
            }),

            DrunTools::SessionCommitTool(t) => self.with_session(&t.session_id, |session| {
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
            }),

            DrunTools::SessionTreeTool(_) => {
                let sessions = self.sessions.lock().unwrap().clone();
                Ok(text(build_session_tree(&sessions)))
            }

            DrunTools::ListSnapshotsTool(_) => {
                Ok(text(build_snapshot_catalog(&self.config.snapshots_dir)))
            }

            DrunTools::SessionExportTool(t) => {
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

            DrunTools::SessionFetchTool(t) => {
                if !self.sessions.lock().unwrap().contains_key(&t.session_id) {
                    return Err(DrunError::session_not_found(&t.session_id).into_tool_err());
                }
                let url_is_allowed =
                    host_from_url(&t.url).map_or(false, |h| self.config.domain_allowed(&h));
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
                loop {
                    match response
                        .chunk()
                        .await
                        .map_err(|e| DrunError::internal(e).into_tool_err())?
                    {
                        Some(chunk) => {
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
                        None => break,
                    }
                }

                let save_path = t.save_to.unwrap_or_else(|| download_path_from_url(&t.url));
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

            DrunTools::GetFetchAllowlistTool(_) => Ok(text(
                serde_json::to_string(&self.config.domain_allowlist).unwrap(),
            )),

            DrunTools::SessionSnapshotTool(t) => {
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

            DrunTools::SessionGetEnvTool(t) => {
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

            DrunTools::SessionRestoreTool(t) => {
                let bytes =
                    std::fs::read(&t.path).map_err(|e| DrunError::internal(e).into_tool_err())?;
                let snapshot = SessionSnapshot::decode(&bytes)
                    .map_err(|e| DrunError::internal(e).into_tool_err())?;
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

            DrunTools::SessionLabelTool(t) => self.with_session_mut(&t.session_id, |session| {
                session.set_label(t.label);
                Ok(text(build_session_state(
                    &t.session_id,
                    session,
                    None,
                    vec![],
                )))
            }),

            DrunTools::SessionCheckpointLabelTool(t) => {
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

            DrunTools::SessionCheckpointSquashTool(t) => {
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

            DrunTools::SessionMergeTool(t) => {
                let source_arc = self
                    .sessions
                    .lock()
                    .unwrap()
                    .get(&t.source_session_id)
                    .ok_or_else(|| {
                        DrunError::session_not_found(&t.source_session_id).into_tool_err()
                    })?
                    .clone();
                let source = source_arc.lock().unwrap();
                let source_checkpoint_id =
                    match (t.source_checkpoint_id, t.source_checkpoint_label.as_deref()) {
                        (_, Some(lbl)) => {
                            Some(source.checkpoint_by_label(lbl).ok_or_else(|| {
                                DrunError::internal(format!("no checkpoint with label '{lbl}'"))
                                    .into_tool_err()
                            })?)
                        }
                        (Some(id), None) => Some(id as usize),
                        (None, None) => None,
                    };
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

            DrunTools::SessionCheckpointDropTool(t) => {
                self.with_session_mut(&t.session_id, |session| {
                    session
                        .drop_checkpoints(
                            t.from_checkpoint_id as usize,
                            t.to_checkpoint_id as usize,
                        )
                        .map_err(|e| DrunError::internal(e).into_tool_err())?;
                    Ok(text(build_checkpoint_history(session)))
                })
            }

            DrunTools::CheckpointReadStdstreamsTool(t) => {
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
        }
    }
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
        authority.to_string()
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

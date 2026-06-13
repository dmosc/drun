use crate::errors::DrunError;
use crate::handler::DrunHandler;
use crate::response::{file_content, text};
use crate::state::{
    build_checkpoint_history, build_session_list, build_session_state, build_session_tree,
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
        ProgressNotificationParams, RpcError, schema_utils::CallToolError,
    },
};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

const DEFAULT_SNAPSHOTS_FOLDER: &str = "drun-snapshots";

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
            DrunTools::CreateSessionTool(t) => {
                if let Some(max) = self.max_sessions {
                    if self.sessions.lock().unwrap().len() >= max {
                        return Err(DrunError::session_limit_reached(max).into_tool_err());
                    }
                }
                let session_id = Uuid::new_v4().to_string();
                let allowed_hosts = self.build_allowed_hosts(t.allowed_hosts);
                let session = Session::new(&self.engine, allowed_hosts, t.timeout_ms)
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
                    Session::from_session(
                        &self.engine,
                        &t.session_id,
                        &source,
                        t.checkpoint_id.map(|id| id as usize),
                    )
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
                if self.auto_snapshot {
                    let output_path = self
                        .snapshots_dir
                        .clone()
                        .unwrap_or_else(|| {
                            std::env::current_dir()
                                .unwrap_or_default()
                                .join(DEFAULT_SNAPSHOTS_FOLDER)
                        })
                        .join(format!("{}.drun", t.session_id));
                    if let Some(parent_dir) = output_path.parent() {
                        let _ = std::fs::create_dir_all(parent_dir);
                    }
                    if let Ok(bytes) = session.lock().unwrap().snapshot().encode() {
                        let _ = std::fs::write(output_path, bytes);
                    }
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

            DrunTools::SessionInstallPackageTool(t) => {
                if !self.allowed_packages.is_empty() && !self.allowed_packages.contains(&t.package)
                {
                    return Err(DrunError::package_denied(&t.package).into_tool_err());
                }
                self.with_session_mut(&t.session_id, |session| {
                    session
                        .install(&t.package)
                        .map_err(|e| DrunError::from_install(&t.package, e).into_tool_err())?;
                    Ok(text(build_session_state(
                        &t.session_id,
                        session,
                        None,
                        vec![],
                    )))
                })
            }

            DrunTools::SessionExecuteTool(t) => {
                let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                {
                    let rt = runtime.clone();
                    let token = progress_token.clone();
                    tokio::spawn(async move {
                        let Some(tok) = token else {
                            return;
                        };
                        let mut rx = progress_rx;
                        while let Some(chunk) = rx.recv().await {
                            let _ = rt
                                .notify_progress(ProgressNotificationParams {
                                    progress: 0.0,
                                    progress_token: tok.clone(),
                                    message: Some(chunk),
                                    total: None,
                                    meta: None,
                                })
                                .await;
                        }
                    });
                }
                self.with_session_mut(&t.session_id, |session| {
                    let previous_files = session.current().files.clone();
                    session
                        .execute(&t.code, &mut |chunk| {
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
                let previous_files = session.current().files.clone();
                session
                    .rollback(t.checkpoint_id as usize)
                    .map_err(|e| DrunError::internal(e).into_tool_err())?;
                Ok(text(build_session_state(
                    &t.session_id,
                    session,
                    Some(&previous_files),
                    vec![],
                )))
            }),

            DrunTools::SessionReadFileTool(t) => self.with_session(&t.session_id, |session| {
                let bytes = session
                    .current()
                    .files
                    .get(&t.path)
                    .ok_or_else(|| DrunError::file_not_found(&t.path).into_tool_err())?;
                Ok(file_content(&t.path, bytes))
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
                let from = t.from_checkpoint_id.unwrap_or(0) as usize;
                let to = t
                    .to_checkpoint_id
                    .map(|id| id as usize)
                    .unwrap_or_else(|| session.current().id);
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

            DrunTools::SessionExportTool(t) => {
                static DEFAULT_EXPORT_FOLDER: &str = "drun-export";
                let output_dir = match &t.output_dir {
                    Some(dir) => {
                        let p = PathBuf::from(dir);
                        if let Some(ref root) = self.export_root {
                            if !p.starts_with(root) {
                                return Err(DrunError::export_denied(
                                    &p.display().to_string(),
                                    &root.display().to_string(),
                                )
                                .into());
                            }
                        }
                        p
                    }
                    None => self
                        .export_root
                        .clone()
                        .unwrap_or_else(|| {
                            std::env::current_dir()
                                .unwrap_or_default()
                                .join(DEFAULT_EXPORT_FOLDER)
                        })
                        .join(&t.session_id),
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
                let url_is_allowed = self.fetch_allowlist.iter().any(|h| h == "*")
                    || host_from_url(&t.url).map_or(false, |h| self.fetch_allowlist.contains(&h));
                if !url_is_allowed {
                    return Err(DrunError::fetch_denied(&t.url).into_tool_err());
                }

                let method = t.method.as_deref().unwrap_or("GET").to_uppercase();
                let parsed_method = method.parse::<reqwest::Method>().map_err(|_| {
                    DrunError::internal(format!("invalid HTTP method: {method}")).into_tool_err()
                })?;

                // 30 s connect guard is hardcoded; overall timeout only if operator configured it.
                let mut builder =
                    reqwest::Client::builder().connect_timeout(Duration::from_secs(30));
                if let Some(ms) = self.fetch_timeout_ms {
                    builder = builder.timeout(Duration::from_millis(ms));
                }
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

                let response = req
                    .send()
                    .await
                    .map_err(|e| DrunError::internal(e).into_tool_err())?;
                let status = response.status().as_u16();
                let response_headers: HashMap<String, String> = response
                    .headers()
                    .iter()
                    .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
                    .collect();
                let body_bytes = response
                    .bytes()
                    .await
                    .map_err(|e| DrunError::internal(e).into_tool_err())?;
                let (body, body_encoding) = match std::str::from_utf8(&body_bytes) {
                    Ok(t) => (t.to_string(), None),
                    Err(_) => (BASE64.encode(&body_bytes), Some("base64")),
                };

                Ok(text(
                    serde_json::to_string(&FetchResponse {
                        status,
                        headers: response_headers,
                        body,
                        body_encoding,
                    })
                    .unwrap(),
                ))
            }

            DrunTools::GetFetchAllowlistTool(_) => {
                Ok(text(serde_json::to_string(&self.fetch_allowlist).unwrap()))
            }

            DrunTools::SessionSnapshotTool(t) => {
                let output_path = match t.path {
                    Some(p) => PathBuf::from(p),
                    None => self
                        .snapshots_dir
                        .clone()
                        .unwrap_or_else(|| {
                            std::env::current_dir()
                                .unwrap_or_default()
                                .join(DEFAULT_SNAPSHOTS_FOLDER)
                        })
                        .join(format!("{}.drun", t.session_id)),
                };
                if let Some(parent_dir) = output_path.parent() {
                    std::fs::create_dir_all(parent_dir)
                        .map_err(|e| DrunError::internal(e).into_tool_err())?;
                }
                self.with_session(&t.session_id, |session| {
                    let encoded = session
                        .snapshot()
                        .encode()
                        .map_err(|e| DrunError::internal(e).into_tool_err())?;
                    std::fs::write(&output_path, encoded)
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
                if !self.env_allowlist.contains(&t.name) {
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
                let restored = Session::from_snapshot(&self.engine, snapshot)
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
        }
    }
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

#[derive(Serialize)]
struct FetchResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_encoding: Option<&'static str>,
}

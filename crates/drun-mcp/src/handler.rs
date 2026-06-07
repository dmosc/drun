//! MCP request handler. Owns the session map and dispatches each tool call to
//! the appropriate session operation.

use crate::response::{err, file_content, text};
use crate::tools::DrunTools;
use async_trait::async_trait;
use drun_core::{DrunEngine, NetworkPolicy, Session};
use rust_mcp_sdk::{
    McpServer,
    mcp_server::ServerHandler,
    schema::{
        CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams, RpcError,
        schema_utils::CallToolError,
    },
};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

pub struct DrunHandler {
    sessions: Mutex<HashMap<String, Session>>,
}

impl DrunHandler {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

fn get_session_state_delta(
    previous_files: Option<&HashMap<String, Vec<u8>>>,
    current_files: &HashMap<String, Vec<u8>>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut removed = Vec::new();
    if let Some(previous_files) = previous_files {
        for key in current_files.keys() {
            if !previous_files.contains_key(key) {
                added.push(key.clone());
            } else if current_files[key] != previous_files[key] {
                modified.push(key.clone());
            }
        }
        for key in previous_files.keys() {
            if !current_files.contains_key(key) {
                removed.push(key.clone());
            }
        }
        added.sort();
        modified.sort();
        removed.sort();
    }
    (added, modified, removed)
}

#[derive(Serialize)]
struct SessionState {
    session_id: String,
    checkpoint_id: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    stdout: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    stderr: String,
    workspace: Vec<String>,
    packages: Vec<String>,
    timeout_ms: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files_added: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files_modified: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files_removed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    committed_files: Vec<String>,
}

fn build_state(
    session_id: &str,
    session: &Session,
    previous_files: Option<&HashMap<String, Vec<u8>>>,
    committed_files: Vec<String>,
) -> String {
    let current = session.current();
    let mut workspace: Vec<String> = current.files.keys().cloned().collect();
    workspace.sort();
    let (files_added, files_modified, files_removed) =
        get_session_state_delta(previous_files, &current.files);
    serde_json::to_string(&SessionState {
        session_id: session_id.to_string(),
        checkpoint_id: current.id,
        stdout: current.stdout.clone(),
        stderr: current.stderr.clone(),
        workspace,
        packages: session.packages().to_vec(),
        timeout_ms: session.timeout_ms,
        files_added,
        files_modified,
        files_removed,
        committed_files,
    })
    .unwrap()
}

#[derive(Serialize)]
struct CheckpointSummary {
    checkpoint_id: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    stdout: String,
    file_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files_added: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files_modified: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files_removed: Vec<String>,
}

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
        _runtime: Arc<dyn McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        match DrunTools::try_from(params)? {
            DrunTools::CreateSessionTool(t) => {
                let id = Uuid::new_v4().to_string();
                let engine = DrunEngine::new().map_err(err)?;
                let network = match t.network.as_deref() {
                    Some("full") => NetworkPolicy::Full,
                    Some("none") => NetworkPolicy::None,
                    _ => NetworkPolicy::Packages,
                };
                let session = Session::new(&engine, network, t.timeout_ms).map_err(err)?;
                let state = build_state(&id, &session, None, vec![]);
                self.sessions.lock().unwrap().insert(id, session);
                Ok(text(state))
            }
            DrunTools::SessionListTool(_) => {
                let sessions = self.sessions.lock().unwrap();
                let list: Vec<serde_json::Value> = sessions
                    .iter()
                    .map(|(id, s)| {
                        serde_json::json!({
                            "session_id": id,
                            "checkpoint_count": s.history().len(),
                            "packages": s.packages(),
                            "timeout_ms": s.timeout_ms,
                        })
                    })
                    .collect();
                Ok(text(serde_json::to_string(&list).unwrap()))
            }
            DrunTools::SessionCloseTool(t) => {
                let session = self.sessions.lock().unwrap().remove(&t.session_id);
                if session.is_none() {
                    return Err(err(format!("session '{}' not found", t.session_id)));
                }
                Ok(text(format!("closed {}", t.session_id)))
            }
            DrunTools::SessionHistoryTool(t) => {
                let sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                let history = session.history();
                let summaries: Vec<CheckpointSummary> = history
                    .iter()
                    .enumerate()
                    .map(|(i, cp)| {
                        let prev = if i > 0 {
                            Some(&history[i - 1].files)
                        } else {
                            None
                        };
                        let (files_added, files_modified, files_removed) =
                            get_session_state_delta(prev, &cp.files);
                        CheckpointSummary {
                            checkpoint_id: cp.id,
                            stdout: cp.stdout.clone(),
                            file_count: cp.files.len(),
                            files_added,
                            files_modified,
                            files_removed,
                        }
                    })
                    .collect();
                Ok(text(serde_json::to_string(&summaries).unwrap()))
            }
            DrunTools::GetSessionStateTool(t) => {
                let sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                Ok(text(build_state(&t.session_id, session, None, vec![])))
            }
            DrunTools::SessionInstallPackageTool(t) => {
                let mut sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get_mut(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                session.install(&t.package).map_err(err)?;
                Ok(text(build_state(&t.session_id, session, None, vec![])))
            }
            DrunTools::SessionExecuteTool(t) => {
                let mut sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get_mut(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                session.execute(&t.code).map_err(err)?;
                let current_id = session.current().id;
                let previous_files = if current_id > 0 {
                    Some(&session.history()[current_id - 1].files)
                } else {
                    None
                };
                Ok(text(build_state(
                    &t.session_id,
                    session,
                    previous_files,
                    vec![],
                )))
            }
            DrunTools::SessionRollbackTool(t) => {
                let mut sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get_mut(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                let previous_files = session.current().files.clone();
                session.rollback(t.checkpoint_id as usize).map_err(err)?;
                Ok(text(build_state(
                    &t.session_id,
                    session,
                    Some(&previous_files),
                    vec![],
                )))
            }
            DrunTools::SessionReadFileTool(t) => {
                let sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                let bytes = session
                    .current()
                    .files
                    .get(&t.path)
                    .ok_or_else(|| err(format!("'{}' not in current checkpoint", t.path)))?;
                Ok(file_content(&t.path, bytes))
            }
            DrunTools::SessionWriteFileTool(t) => {
                let mut sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get_mut(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                let previous_files = session.current().files.clone();
                session.write_file(&t.path, t.content.into_bytes());
                Ok(text(build_state(
                    &t.session_id,
                    session,
                    Some(&previous_files),
                    vec![],
                )))
            }
            DrunTools::SessionDeleteFileTool(t) => {
                let mut sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get_mut(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                let previous_files = session.current().files.clone();
                session.delete_file(&t.path).map_err(err)?;
                Ok(text(build_state(
                    &t.session_id,
                    session,
                    Some(&previous_files),
                    vec![],
                )))
            }
            DrunTools::SessionDiffTool(t) => {
                let sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                let from = t.from_checkpoint_id.unwrap_or(0) as usize;
                let to = t
                    .to_checkpoint_id
                    .map(|id| id as usize)
                    .unwrap_or_else(|| session.current().id);
                let diff = session.diff(from, to).map_err(err)?;
                Ok(text(if diff.is_empty() {
                    "no changes".into()
                } else {
                    diff
                }))
            }
            DrunTools::SessionMountTool(t) => {
                let mut sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get_mut(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                let previous_files = session.current().files.clone();
                session.mount(std::path::Path::new(&t.path)).map_err(err)?;
                Ok(text(build_state(
                    &t.session_id,
                    session,
                    Some(&previous_files),
                    vec![],
                )))
            }
            DrunTools::SessionCommitTool(t) => {
                let sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                let paths = session.commit(t.keys).map_err(err)?;
                let committed_files = paths
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                Ok(text(build_state(
                    &t.session_id,
                    session,
                    None,
                    committed_files,
                )))
            }
            DrunTools::SessionExportTool(t) => {
                static DEFAULT_DRUN_EXPORT_FOLDER: &str = "drun-export";
                let sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                let output_dir = match &t.output_dir {
                    Some(d) => std::path::PathBuf::from(d),
                    None => std::env::current_dir()
                        .map_err(err)?
                        .join(DEFAULT_DRUN_EXPORT_FOLDER)
                        .join(&t.session_id),
                };
                let exported_files = session.export(&output_dir, t.keys).map_err(err)?;
                let exported_files: Vec<String> = exported_files
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                Ok(text(
                    serde_json::json!({
                        "output_dir": output_dir.to_string_lossy(),
                        "exported_files": exported_files,
                    })
                    .to_string(),
                ))
            }
        }
    }
}

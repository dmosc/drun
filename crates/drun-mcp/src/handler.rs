//! MCP request handler. Owns the session map and dispatches each tool call to
//! the appropriate session operation.

use crate::response::{err, file_content, text};
use crate::state::{build_checkpoint_history, build_session_state};
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
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

fn parse_network_policy(s: Option<&str>) -> NetworkPolicy {
    match s {
        Some("full") => NetworkPolicy::Full,
        Some("none") => NetworkPolicy::None,
        _ => NetworkPolicy::Packages,
    }
}

pub struct DrunHandler {
    engine: DrunEngine,
    sessions: Mutex<HashMap<String, Session>>,
}

impl DrunHandler {
    pub fn new() -> Self {
        Self {
            engine: DrunEngine::new().expect("failed to initialize drun engine"),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn with_session(
        &self,
        session_id: &str,
        f: impl FnOnce(&Session) -> Result<CallToolResult, CallToolError>,
    ) -> Result<CallToolResult, CallToolError> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(session_id)
            .ok_or_else(|| err(format!("session '{}' not found", session_id)))?;
        f(session)
    }

    fn with_session_mut(
        &self,
        session_id: &str,
        f: impl FnOnce(&mut Session) -> Result<CallToolResult, CallToolError>,
    ) -> Result<CallToolResult, CallToolError> {
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| err(format!("session '{}' not found", session_id)))?;
        f(session)
    }
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
                let session_id = Uuid::new_v4().to_string();
                let network = parse_network_policy(t.network.as_deref());
                let session = Session::new(&self.engine, network, t.timeout_ms).map_err(err)?;
                let state = build_session_state(&session_id, &session, None, vec![]);
                self.sessions.lock().unwrap().insert(session_id, session);
                Ok(text(state))
            }

            DrunTools::SessionForkTool(t) => {
                let fork = {
                    let sessions = self.sessions.lock().unwrap();
                    let source = sessions
                        .get(&t.session_id)
                        .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                    Session::from_session(
                        &self.engine,
                        &t.session_id,
                        source,
                        t.checkpoint_id.map(|id| id as usize),
                    )
                    .map_err(err)?
                };
                let fork_id = Uuid::new_v4().to_string();
                let state = build_session_state(&fork_id, &fork, None, vec![]);
                self.sessions.lock().unwrap().insert(fork_id, fork);
                Ok(text(state))
            }

            DrunTools::SessionListTool(_) => {
                let sessions = self.sessions.lock().unwrap();
                let list: Vec<serde_json::Value> = sessions
                    .iter()
                    .map(|(id, session)| {
                        let mut entry = serde_json::json!({
                            "session_id": id,
                            "checkpoint_count": session.history().len(),
                            "packages": session.packages(),
                            "timeout_ms": session.timeout_ms,
                        });
                        if let Some(r) = &session.parent {
                            entry["parent_session_id"] = serde_json::json!(r.session_id);
                            entry["parent_checkpoint_id"] = serde_json::json!(r.checkpoint_id);
                        }
                        entry
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
                self.with_session_mut(&t.session_id, |session| {
                    session.install(&t.package).map_err(err)?;
                    Ok(text(build_session_state(
                        &t.session_id,
                        session,
                        None,
                        vec![],
                    )))
                })
            }

            DrunTools::SessionExecuteTool(t) => self.with_session_mut(&t.session_id, |session| {
                let previous_files = session.current().files.clone();
                session.execute(&t.code).map_err(err)?;
                Ok(text(build_session_state(
                    &t.session_id,
                    session,
                    Some(&previous_files),
                    vec![],
                )))
            }),

            DrunTools::SessionRollbackTool(t) => self.with_session_mut(&t.session_id, |session| {
                let previous_files = session.current().files.clone();
                session.rollback(t.checkpoint_id as usize).map_err(err)?;
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
                    .ok_or_else(|| err(format!("'{}' not in current checkpoint", t.path)))?;
                Ok(file_content(&t.path, bytes))
            }),

            DrunTools::SessionWriteFileTool(t) => self.with_session_mut(&t.session_id, |session| {
                let previous_files = session.current().files.clone();
                session.write_file(&t.path, t.content.into_bytes());
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
                    session.delete_file(&t.path).map_err(err)?;
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
                session.mount(std::path::Path::new(&t.path)).map_err(err)?;
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
                let diff = session.diff(from, to).map_err(err)?;
                Ok(text(if diff.is_empty() {
                    "no changes".into()
                } else {
                    diff
                }))
            }),

            DrunTools::SessionCommitTool(t) => self.with_session(&t.session_id, |session| {
                let paths = session.commit(t.keys).map_err(err)?;
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

            DrunTools::SessionExportTool(t) => {
                static DEFAULT_EXPORT_FOLDER: &str = "drun-export";
                self.with_session(&t.session_id, |session| {
                    let output_dir = match &t.output_dir {
                        Some(dir) => std::path::PathBuf::from(dir),
                        None => std::env::current_dir()
                            .map_err(err)?
                            .join(DEFAULT_EXPORT_FOLDER)
                            .join(&t.session_id),
                    };
                    let exported_files = session.export(&output_dir, t.keys).map_err(err)?;
                    let exported_paths: Vec<String> = exported_files
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();
                    Ok(text(
                        serde_json::json!({
                            "output_dir": output_dir.to_string_lossy(),
                            "exported_files": exported_paths,
                        })
                        .to_string(),
                    ))
                })
            }
        }
    }
}

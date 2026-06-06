//! MCP request handler. Owns the session map and dispatches each tool call to
//! the appropriate session operation.

use crate::response::{err, file_content, text};
use crate::tools::DrunTools;
use async_trait::async_trait;
use drun_core::{DrunEngine, NetworkPolicy, Session, read_host_path};
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
                let session = Session::new(&engine, network).map_err(err)?;
                self.sessions.lock().unwrap().insert(id.clone(), session);
                Ok(text(id))
            }
            DrunTools::SessionInstallPackageTool(t) => {
                let mut sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get_mut(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                session
                    .execute(&format!(
                        "import micropip\nawait micropip.install('{}')",
                        t.package
                    ))
                    .map_err(err)?;
                Ok(text(format!("installed {}", t.package)))
            }
            DrunTools::SessionExecuteTool(t) => {
                let mut sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get_mut(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                let checkpoint = session.execute(&t.code).map_err(err)?;
                Ok(text(
                    serde_json::json!({
                        "checkpoint_id": checkpoint.id,
                        "stdout": checkpoint.stdout,
                    })
                    .to_string(),
                ))
            }
            DrunTools::SessionRollbackTool(t) => {
                let mut sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get_mut(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                session.rollback(t.checkpoint_id as usize).map_err(err)?;
                Ok(text(format!(
                    "rolled back to checkpoint {}",
                    t.checkpoint_id
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
            DrunTools::SessionMountTool(t) => {
                let files = read_host_path(std::path::Path::new(&t.path)).map_err(err)?;
                let keys: Vec<String> = files.keys().cloned().collect();
                let mut sessions = self.sessions.lock().unwrap();
                let session = sessions
                    .get_mut(&t.session_id)
                    .ok_or_else(|| err(format!("session '{}' not found", t.session_id)))?;
                session.mount(files);
                Ok(text(format!("mounted: {}", keys.join(", "))))
            }
        }
    }
}

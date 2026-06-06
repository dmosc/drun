use async_trait::async_trait;
use drun_core::{DrunEngine, Session};
use rust_mcp_sdk::{
    McpServer, StdioTransport, ToMcpServerHandler, TransportOptions,
    error::SdkResult,
    macros::{JsonSchema, mcp_tool},
    mcp_server::{McpServerOptions, ServerHandler, server_runtime},
    schema::{
        CallToolRequestParams, CallToolResult, Implementation, InitializeResult, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, RpcError, ServerCapabilities,
        ServerCapabilitiesTools, TextContent, schema_utils::CallToolError,
    },
    tool_box,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

// ── Tools ────────────────────────────────────────────────────────────────────

#[mcp_tool(
    name = "create_session",
    description = "Create a persistent sandbox session. Returns a session_id for subsequent calls.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct CreateSessionTool {}

#[mcp_tool(
    name = "session_execute",
    description = "Run Python code in an existing session, building on the current checkpoint. Returns stdout and the new checkpoint_id.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct SessionExecuteTool {
    /// Session ID from create_session
    session_id: String,
    /// Python source code to run
    code: String,
}

#[mcp_tool(
    name = "session_rollback",
    description = "Roll back a session to a prior checkpoint, discarding all state after it.",
    idempotent_hint = false,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct SessionRollbackTool {
    /// Session ID from create_session.
    session_id: String,
    /// Checkpoint ID to restore.
    checkpoint_id: u64,
}

#[mcp_tool(
    name = "session_install_package",
    description = "Install a Python package into the session. The package will be available in all subsequent session_execute calls.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct SessionInstallPackageTool {
    /// Session ID from create_session
    session_id: String,
    /// Package name as it appears on PyPI (e.g. "pandas" or "faker==1.0.0")
    package: String,
}

#[mcp_tool(
    name = "session_read_file",
    description = "Read the UTF-8 content of a file from the current session checkpoint.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
struct SessionReadFileTool {
    /// Session ID from create_session
    session_id: String,
    /// File path relative to /workspace.
    path: String,
}

tool_box!(
    DrunTools,
    [
        CreateSessionTool,
        SessionInstallPackageTool,
        SessionExecuteTool,
        SessionRollbackTool,
        SessionReadFileTool,
    ]
);

// ── Handler ──────────────────────────────────────────────────────────────────

struct DrunHandler {
    sessions: Mutex<HashMap<String, Session>>,
}

impl DrunHandler {
    fn new() -> Self {
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
            DrunTools::CreateSessionTool(_) => {
                let id = Uuid::new_v4().to_string();
                let engine = DrunEngine::new().map_err(err)?;
                let session = Session::new(&engine).map_err(err)?;
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
                Ok(text(String::from_utf8_lossy(bytes).into_owned()))
            }
        }
    }
}

fn text(s: impl Into<String>) -> CallToolResult {
    CallToolResult::text_content(vec![TextContent::from(s.into())])
}

fn err(e: impl ToString) -> CallToolError {
    CallToolError(e.to_string().into())
}

// ── Entry point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> SdkResult<()> {
    let handler = DrunHandler::new().to_mcp_server_handler();

    let server = server_runtime::create_server(McpServerOptions {
        server_details: InitializeResult {
            server_info: Implementation {
                name: "drun".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: Some("drun".into()),
                description: Some("Sandboxed code execution for agentic loops".into()),
                icons: vec![],
                website_url: None,
            },
            capabilities: ServerCapabilities {
                tools: Some(ServerCapabilitiesTools { list_changed: None }),
                ..Default::default()
            },
            protocol_version: ProtocolVersion::V2025_11_25.into(),
            instructions: Some("Go to https://drun.dev to view docs.".into()),
            meta: None,
        },
        transport: StdioTransport::new(TransportOptions::default())?,
        handler,
        task_store: None,
        client_task_store: None,
        message_observer: None,
    });

    server.start().await
}

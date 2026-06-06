//! MCP tool schemas exposed to clients. Each struct maps to one callable tool
//! with its input parameters and hints.

use rust_mcp_sdk::{
    macros::{JsonSchema, mcp_tool},
    tool_box,
};
use serde::{Deserialize, Serialize};

#[mcp_tool(
    name = "create_session",
    description = "Create a persistent sandbox session. Returns a session_id for subsequent calls.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateSessionTool {
    /// Network access policy: "packages" (default — PyPI only), "full" (unrestricted), or "none".
    pub network: Option<String>,
}

#[mcp_tool(
    name = "session_execute",
    description = "Run Python code in an existing session, building on the current checkpoint. Returns stdout and the new checkpoint_id.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionExecuteTool {
    /// Session ID from create_session
    pub session_id: String,
    /// Python source code to run
    pub code: String,
}

#[mcp_tool(
    name = "session_rollback",
    description = "Roll back a session to a prior checkpoint, discarding all state after it.",
    idempotent_hint = false,
    destructive_hint = true,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionRollbackTool {
    /// Session ID from create_session.
    pub session_id: String,
    /// Checkpoint ID to restore.
    pub checkpoint_id: u64,
}

#[mcp_tool(
    name = "session_install_package",
    description = "Install a Python package into the session. The package will be available in all subsequent session_execute calls.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionInstallPackageTool {
    /// Session ID from create_session
    pub session_id: String,
    /// Package name as it appears on PyPI (e.g. "pandas" or "faker==1.0.0")
    pub package: String,
}

#[mcp_tool(
    name = "session_read_file",
    description = "Read the contents of a file from the current session checkpoint. Works for any file type including text, JSON, images, and other binary formats.",
    idempotent_hint = true,
    destructive_hint = false,
    read_only_hint = true
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionReadFileTool {
    /// Session ID from create_session
    pub session_id: String,
    /// File path relative to /workspace.
    pub path: String,
}

#[mcp_tool(
    name = "session_mount",
    description = "Copy a file or directory from the host filesystem into the session workspace. Files become available at /workspace/<filename> (or /workspace/<relative-path> for directories). Call before session_execute to make host data accessible to the sandbox.",
    idempotent_hint = false,
    destructive_hint = false,
    read_only_hint = false
)]
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SessionMountTool {
    /// Session ID from create_session
    pub session_id: String,
    /// Absolute path to a file or directory on the host filesystem.
    pub path: String,
}

tool_box!(
    DrunTools,
    [
        CreateSessionTool,
        SessionInstallPackageTool,
        SessionExecuteTool,
        SessionRollbackTool,
        SessionReadFileTool,
        SessionMountTool,
    ]
);

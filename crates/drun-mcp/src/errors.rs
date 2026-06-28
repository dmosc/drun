//! DrunError: structured MCP tool error type. Constructors map domain errors
//! (session not found, timeout, command denied, etc.) to typed error codes
//! that clients can inspect programmatically.

use drun_core::RunnerError;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct DrunError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl DrunError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn session_not_found(session_id: &str) -> Self {
        Self::new(
            "session_not_found",
            format!("session '{session_id}' not found"),
        )
    }

    pub fn session_busy(session_id: &str) -> Self {
        Self::new(
            "session_busy",
            format!(
                "session '{session_id}' is currently executing; wait for it to complete and retry"
            ),
        )
    }

    pub fn session_idle(session_id: &str, idle_secs: u64, limit_secs: u64) -> Self {
        Self::new(
            "session_idle",
            format!("session '{session_id}' has been idle for {idle_secs}s (limit: {limit_secs}s); close it and open a new one"),
        )
        .with_detail(serde_json::json!({ "idle_secs": idle_secs, "limit_secs": limit_secs }))
    }

    pub fn session_limit_reached(limit: usize) -> Self {
        Self::new(
            "session_limit_reached",
            format!(
                "session limit of {limit} reached; close an existing session to create a new one"
            ),
        )
        .with_detail(serde_json::json!({ "limit": limit }))
    }

    pub fn execution_timeout(timeout_ms: u64) -> Self {
        Self::new(
            "execution_timeout",
            format!("execution exceeded the {timeout_ms}ms timeout; increase bash_timeout_ms in server config"),
        )
        .with_detail(serde_json::json!({ "timeout_ms": timeout_ms }))
    }

    pub fn command_denied(message: impl Into<String>) -> Self {
        Self::new("command_denied", message)
    }

    pub fn fetch_denied(url: &str) -> Self {
        Self::new(
            "fetch_denied",
            format!("'{url}' is not permitted by the server's fetch allowlist"),
        )
    }

    pub fn file_not_found(path: &str) -> Self {
        Self::new(
            "file_not_found",
            format!("'{path}' not found in current checkpoint"),
        )
    }

    pub fn export_denied(path: &str, allowed_root: &str) -> Self {
        Self::new(
            "export_denied",
            format!("export to '{path}' is not permitted; must be under '{allowed_root}'"),
        )
    }

    pub fn snapshot_denied(path: &str, allowed_root: &str) -> Self {
        Self::new(
            "snapshot_denied",
            format!("snapshot to '{path}' is not permitted; must be under '{allowed_root}'"),
        )
    }

    pub fn env_var_denied(name: &str) -> Self {
        Self::new(
            "env_var_denied",
            format!("'{name}' is not in the server's env_allowlist"),
        )
    }

    pub fn internal(message: impl ToString) -> Self {
        Self::new("internal_error", message.to_string())
    }

    pub fn from_exec(e: anyhow::Error) -> Self {
        match e.downcast_ref::<RunnerError>() {
            Some(RunnerError::Timeout { timeout_ms }) => Self::execution_timeout(*timeout_ms),
            Some(RunnerError::CommandDenied(msg)) => Self::command_denied(msg.clone()),
            None => Self::internal(e),
        }
    }

    pub fn into_tool_err(self) -> CallToolError {
        CallToolError::from(self)
    }
}

impl From<DrunError> for CallToolError {
    fn from(e: DrunError) -> Self {
        let body = serde_json::to_string(&e).unwrap_or_else(|_| e.message.clone());
        CallToolError(body.into())
    }
}

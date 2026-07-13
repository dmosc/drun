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

    pub fn checkpoint_not_found(message: impl Into<String>) -> Self {
        Self::new("checkpoint_not_found", message)
    }

    pub fn checkpoint_limit_reached(limit: usize) -> Self {
        Self::new(
            "checkpoint_limit_reached",
            format!(
                "checkpoint limit of {limit} reached; close or snapshot this session and start a new one"
            ),
        )
        .with_detail(serde_json::json!({ "limit": limit }))
    }

    pub fn invalid_workspace_path(message: impl Into<String>) -> Self {
        Self::new("invalid_workspace_path", message)
    }

    pub fn mount_denied(message: impl Into<String>) -> Self {
        Self::new("mount_denied", message)
    }

    pub fn workspace_size_exceeded(actual_bytes: u64, limit_bytes: u64) -> Self {
        Self::new(
            "workspace_size_exceeded",
            format!("workspace size {actual_bytes} bytes exceeds limit of {limit_bytes} bytes"),
        )
        .with_detail(
            serde_json::json!({ "actual_bytes": actual_bytes, "limit_bytes": limit_bytes }),
        )
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
            Some(RunnerError::CheckpointNotFound(msg)) => Self::checkpoint_not_found(msg.clone()),
            Some(RunnerError::CheckpointLimitReached(max)) => Self::checkpoint_limit_reached(*max),
            Some(RunnerError::FileNotFound(msg)) => Self::new("file_not_found", msg.clone()),
            Some(RunnerError::InvalidWorkspacePath(msg)) => {
                Self::invalid_workspace_path(msg.clone())
            }
            Some(RunnerError::MountDenied(msg)) => Self::mount_denied(msg.clone()),
            Some(RunnerError::WorkspaceSizeExceeded {
                actual_bytes,
                limit_bytes,
            }) => Self::workspace_size_exceeded(*actual_bytes, *limit_bytes),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_not_found_has_stable_code_and_mentions_the_id() {
        let err = DrunError::session_not_found("abc123");
        assert_eq!(err.code, "session_not_found");
        assert!(err.message.contains("abc123"));
    }

    #[test]
    fn session_idle_attaches_idle_and_limit_seconds_as_detail() {
        let err = DrunError::session_idle("abc123", 120, 60);
        assert_eq!(err.code, "session_idle");
        assert_eq!(
            err.detail,
            Some(serde_json::json!({ "idle_secs": 120, "limit_secs": 60 }))
        );
    }

    #[test]
    fn session_limit_reached_attaches_limit_as_detail() {
        let err = DrunError::session_limit_reached(50);
        assert_eq!(err.code, "session_limit_reached");
        assert_eq!(err.detail, Some(serde_json::json!({ "limit": 50 })));
    }

    #[test]
    fn most_constructors_have_no_detail_payload() {
        assert_eq!(DrunError::session_not_found("x").detail, None);
        assert_eq!(DrunError::fetch_denied("http://x").detail, None);
        assert_eq!(DrunError::file_not_found("a.txt").detail, None);
    }

    #[test]
    fn from_exec_maps_timeout_to_execution_timeout_code() {
        let err = anyhow::Error::from(RunnerError::Timeout { timeout_ms: 3000 });
        let drun_err = DrunError::from_exec(err);
        assert_eq!(drun_err.code, "execution_timeout");
        assert_eq!(
            drun_err.detail,
            Some(serde_json::json!({ "timeout_ms": 3000 }))
        );
    }

    #[test]
    fn from_exec_maps_command_denied_to_command_denied_code() {
        let err = anyhow::Error::from(RunnerError::CommandDenied("blocked: rm".to_string()));
        let drun_err = DrunError::from_exec(err);
        assert_eq!(drun_err.code, "command_denied");
        assert_eq!(drun_err.message, "blocked: rm");
    }

    #[test]
    fn from_exec_falls_back_to_internal_error_for_unrecognized_errors() {
        let err = anyhow::anyhow!("some unrelated failure");
        let drun_err = DrunError::from_exec(err);
        assert_eq!(drun_err.code, "internal_error");
    }
}

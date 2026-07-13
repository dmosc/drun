#[derive(Debug)]
pub enum RunnerError {
    Timeout { timeout_ms: u64 },
    CommandDenied(String),
    CheckpointNotFound(String),
    CheckpointLimitReached(usize),
    FileNotFound(String),
    InvalidWorkspacePath(String),
    MountDenied(String),
    WorkspaceSizeExceeded { actual_bytes: u64, limit_bytes: u64 },
}

impl RunnerError {
    pub fn timeout(timeout_ms: u64) -> Self {
        Self::Timeout { timeout_ms }
    }

    pub fn command_denied(message: impl Into<String>) -> Self {
        Self::CommandDenied(message.into())
    }

    pub fn checkpoint_not_found(id: usize) -> Self {
        Self::CheckpointNotFound(format!("checkpoint {id} does not exist"))
    }

    pub fn checkpoint_label_not_found(label: &str) -> Self {
        Self::CheckpointNotFound(format!("no checkpoint with label '{label}'"))
    }

    pub fn checkpoint_limit_reached(max: usize) -> Self {
        Self::CheckpointLimitReached(max)
    }

    pub fn file_not_found_in_current(key: &str) -> Self {
        Self::FileNotFound(format!("'{key}' not in current checkpoint"))
    }

    pub fn file_not_found_in_source(key: &str) -> Self {
        Self::FileNotFound(format!("file '{key}' not found in source checkpoint"))
    }

    pub fn invalid_workspace_path(message: impl Into<String>) -> Self {
        Self::InvalidWorkspacePath(message.into())
    }

    pub fn mount_denied(message: impl Into<String>) -> Self {
        Self::MountDenied(message.into())
    }

    pub fn workspace_size_exceeded(actual_bytes: u64, limit_bytes: u64) -> Self {
        Self::WorkspaceSizeExceeded {
            actual_bytes,
            limit_bytes,
        }
    }
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout { timeout_ms } => write!(f, "execution timed out after {timeout_ms}ms"),
            Self::CommandDenied(msg) => write!(f, "{msg}"),
            Self::CheckpointNotFound(msg) => write!(f, "{msg}"),
            Self::CheckpointLimitReached(max) => write!(
                f,
                "checkpoint limit of {max} reached; close or snapshot this session and start a new one"
            ),
            Self::FileNotFound(msg) => write!(f, "{msg}"),
            Self::InvalidWorkspacePath(msg) => write!(f, "{msg}"),
            Self::MountDenied(msg) => write!(f, "{msg}"),
            Self::WorkspaceSizeExceeded {
                actual_bytes,
                limit_bytes,
            } => write!(
                f,
                "workspace size {actual_bytes} bytes exceeds limit of {limit_bytes} bytes"
            ),
        }
    }
}

impl std::error::Error for RunnerError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_display_includes_timeout_ms() {
        let err = RunnerError::Timeout { timeout_ms: 5000 };
        assert_eq!(err.to_string(), "execution timed out after 5000ms");
    }

    #[test]
    fn command_denied_display_is_the_inner_message() {
        let err = RunnerError::CommandDenied("rm -rf / blocked".to_string());
        assert_eq!(err.to_string(), "rm -rf / blocked");
    }

    #[test]
    fn checkpoint_limit_reached_display_includes_the_limit() {
        let err = RunnerError::CheckpointLimitReached(50);
        assert!(err.to_string().contains("50"));
    }

    #[test]
    fn workspace_size_exceeded_display_includes_both_sizes() {
        let err = RunnerError::WorkspaceSizeExceeded {
            actual_bytes: 200,
            limit_bytes: 100,
        };
        assert_eq!(
            err.to_string(),
            "workspace size 200 bytes exceeds limit of 100 bytes"
        );
    }
}

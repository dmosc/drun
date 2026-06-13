/// Typed errors for the cases where the MCP layer needs to distinguish
/// failure modes. Everything else propagates as anyhow::Error.

#[derive(Debug)]
pub enum RunnerError {
    Timeout { timeout_ms: u64 },
    Crash { exit_code: Option<i32> },
    Application(String),
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout { timeout_ms } => write!(f, "execution timed out after {timeout_ms}ms"),
            Self::Crash { exit_code } => {
                write!(
                    f,
                    "sandbox process exited unexpectedly (exit code: {exit_code:?})"
                )
            }
            Self::Application(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for RunnerError {}

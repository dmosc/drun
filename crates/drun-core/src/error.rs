#[derive(Debug)]
pub enum RunnerError {
    Timeout { timeout_ms: u64 },
    CommandDenied(String),
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout { timeout_ms } => write!(f, "execution timed out after {timeout_ms}ms"),
            Self::CommandDenied(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for RunnerError {}

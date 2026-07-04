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
}

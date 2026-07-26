use drun_core::{Config, Session};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct DaemonStatus {
    version: &'static str,
    pid: u32,
    uptime_secs: u64,
    mcp_port: u16,
    web_port: u16,
    session_count: usize,
    max_sessions: Option<usize>,
    session_idle_timeout_secs: Option<u64>,
    max_workspace_mb: Option<u64>,
    max_checkpoints: Option<usize>,
    domain_allowlist: Vec<String>,
    mount_allowlist: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_rss_bytes: Option<u64>,
}

impl DaemonStatus {
    pub(crate) fn current(
        sessions: &HashMap<String, Arc<Mutex<Session>>>,
        config: &Config,
        started_at: Instant,
        mcp_port: u16,
        web_port: u16,
    ) -> DaemonStatus {
        DaemonStatus {
            version: env!("CARGO_PKG_VERSION"),
            pid: std::process::id(),
            uptime_secs: started_at.elapsed().as_secs(),
            mcp_port,
            web_port,
            session_count: sessions.len(),
            max_sessions: config.max_sessions,
            session_idle_timeout_secs: config.session_idle_timeout_secs,
            max_workspace_mb: config.max_workspace_mb,
            max_checkpoints: config.max_checkpoints,
            domain_allowlist: config.domain_allowlist.clone(),
            mount_allowlist: config
                .mount_allowlist
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            memory_rss_bytes: Self::memory_rss_bytes(),
        }
    }

    fn memory_rss_bytes() -> Option<u64> {
        let usage: libc::rusage = unsafe {
            let mut usage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &mut usage) != 0 {
                return None;
            }
            usage
        };
        // ru_maxrss is bytes on macOS, kilobytes on Linux.
        let maxrss = usage.ru_maxrss as u64;
        Some(if cfg!(target_os = "macos") {
            maxrss
        } else {
            maxrss * 1024
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_status_current_reports_session_count_and_config_limits() {
        let config = Config {
            max_sessions: Some(10),
            ..Config::default()
        };
        let mut sessions = HashMap::new();
        sessions.insert(
            "s1".to_string(),
            Arc::new(Mutex::new(Session::new(Config::default().into()).unwrap())),
        );

        let status = DaemonStatus::current(
            &sessions,
            &config,
            Instant::now(),
            crate::Env::DEFAULT_MCP_PORT,
            7274,
        );
        assert_eq!(status.session_count, 1);
        assert_eq!(status.max_sessions, Some(10));
        assert_eq!(status.mcp_port, crate::Env::DEFAULT_MCP_PORT);
        assert_eq!(status.web_port, 7274);
        assert_eq!(status.pid, std::process::id());
    }

    #[test]
    fn memory_rss_bytes_returns_a_plausible_value_on_this_platform() {
        assert!(DaemonStatus::memory_rss_bytes().unwrap_or(1) > 0);
    }
}

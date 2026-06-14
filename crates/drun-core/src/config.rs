use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    /// Domains permitted for session_fetch calls. Use ["*"] to allow all.
    pub domain_allowlist: Vec<String>,
    /// Timeout for session_fetch HTTP requests in milliseconds.
    pub fetch_timeout_ms: u64,
    /// Timeout for Python code execution in milliseconds.
    pub exec_timeout_ms: u64,
    /// Timeout for session_install_package calls in milliseconds.
    pub install_timeout_ms: u64,
    /// Maximum workspace size in megabytes per session.
    pub max_workspace_mb: Option<u64>,
    /// Host path prefixes that may be mounted into a session. Empty means all paths are permitted.
    pub mount_allowlist: Vec<PathBuf>,
    /// Directory that session exports must be written to.
    pub export_root: PathBuf,
    /// Directory where session_snapshot writes .drun files.
    pub snapshots_dir: PathBuf,
    /// Automatically write a .drun snapshot when session_close is called.
    pub auto_snapshot: bool,
    /// Maximum number of concurrent sessions.
    pub max_sessions: Option<usize>,
    /// Maximum number of checkpoints per session.
    pub max_checkpoints: Option<usize>,
    /// Seconds of inactivity after which a session is considered abandoned.
    pub session_idle_timeout_secs: Option<u64>,
    /// Environment variable names the host exposes to agents via session_get_env.
    pub env_allowlist: Vec<String>,
    /// Package names permitted for session_install_package. Empty means all packages are allowed.
    pub package_allowlist: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            domain_allowlist: vec![
                "cdn.jsdelivr.net".to_string(),
                "files.pythonhosted.org".to_string(),
                "pypi.org".to_string(),
            ],
            fetch_timeout_ms: 60_000,
            exec_timeout_ms: 60_000,
            install_timeout_ms: 120_000,
            max_workspace_mb: Some(512),
            max_sessions: Some(50),
            max_checkpoints: Some(200),
            session_idle_timeout_secs: Some(3600),
            mount_allowlist: vec![],
            export_root: PathBuf::from("drun-export"),
            snapshots_dir: PathBuf::from("drun-snapshots"),
            auto_snapshot: false,
            env_allowlist: vec![],
            package_allowlist: vec![],
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let Some(path) = std::env::var("DRUN_CONFIG").ok().map(PathBuf::from) else {
            return Self::default();
        };
        match std::fs::read_to_string(&path).map(|s| toml::from_str::<Config>(&s)) {
            Ok(Ok(config)) => config,
            Ok(Err(e)) => {
                eprintln!("drun: failed to parse config at {}: {e}", path.display());
                Self::default()
            }
            Err(e) => {
                eprintln!("drun: failed to read config at {}: {e}", path.display());
                Self::default()
            }
        }
    }
}

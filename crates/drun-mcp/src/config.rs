use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub fetch: FetchConfig,
    #[serde(default)]
    pub session: SessionConfig,
}

#[derive(Deserialize, Default)]
pub struct FetchConfig {
    /// Domains permitted for session_fetch calls. Use ["*"] to allow all.
    #[serde(default)]
    pub allowlist: Vec<String>,
    /// Overall request timeout in milliseconds. Unset means no limit.
    pub timeout_ms: Option<u64>,
}

#[derive(Deserialize, Default)]
pub struct SessionConfig {
    /// Maximum workspace size in megabytes per session. Unset means no limit.
    pub max_workspace_mb: Option<u64>,
    /// Host path prefixes that may be mounted into a session. Empty means all paths are permitted.
    #[serde(default)]
    pub mount_allowlist: Vec<String>,
    /// Directory that session exports must be written to. Unset means no restriction.
    pub export_root: Option<String>,
    /// Directory where session_snapshot writes .drun files. Unset means no restriction.
    pub snapshots_dir: Option<String>,
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

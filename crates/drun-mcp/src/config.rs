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
}

#[derive(Deserialize, Default)]
pub struct SessionConfig {
    /// Maximum workspace size in megabytes per session. Unset means no limit.
    pub max_workspace_mb: Option<u64>,
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

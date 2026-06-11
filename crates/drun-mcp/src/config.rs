use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub fetch: FetchConfig,
}

#[derive(Deserialize, Default)]
pub struct FetchConfig {
    /// URL prefixes allowed for session_fetch. Use ["*"] to allow all URLs.
    #[serde(default)]
    pub allowlist: Vec<String>,
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

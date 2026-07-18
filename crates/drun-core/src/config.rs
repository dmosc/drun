//! Operator configuration loaded from a TOML file at DRUN_CONFIG. All fields
//! have defaults so the server runs without any config file.

use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Domains permitted for session_fetch calls. Use ["*"] to allow all.
    pub domain_allowlist: Vec<String>,
    /// Timeout for session_fetch HTTP requests in milliseconds (full response).
    pub fetch_timeout_ms: u64,
    /// Timeout for establishing a TCP connection during session_fetch in
    /// milliseconds.
    pub connect_timeout_ms: u64,
    /// Maximum workspace size in megabytes per session.
    pub max_workspace_mb: Option<u64>,
    /// Host path prefixes that may be mounted into a session. Empty means all
    /// paths are permitted.
    pub mount_allowlist: Vec<PathBuf>,
    /// Directory names that session_mount treats as read-only host overlays
    /// instead of loading into the workspace. Symlinked at execution time and
    /// never checkpointed. Set to [] to disable auto-detection.
    pub mount_overlay_paths: Vec<String>,
    /// Directory that session exports must be written to.
    pub export_root: PathBuf,
    /// Directory where session_snapshot writes .drun files.
    pub snapshots_dir: PathBuf,
    /// Automatically write a .drun snapshot when session_close is called.
    pub snapshot_on_close: bool,
    /// Maximum number of concurrent sessions.
    pub max_sessions: Option<usize>,
    /// Maximum number of checkpoints per session.
    pub max_checkpoints: Option<usize>,
    /// Seconds of inactivity after which a session is considered abandoned.
    pub session_idle_timeout_secs: Option<u64>,
    /// Environment variable names the host exposes to agents via
    /// session_get_env.
    pub env_allowlist: Vec<String>,
    /// Timeout for session_bash calls in milliseconds.
    pub bash_timeout_ms: u64,
    /// Shell command substrings that are always denied.
    pub bash_command_denylist: Vec<String>,
    /// Shell command substrings that are permitted. Empty means all commands
    /// are allowed (except for the ones listed in denylist).
    pub bash_command_allowlist: Vec<String>,
    /// TCP port for the embedded web UI. Set to None to disable the web server.
    pub web_port: Option<u16>,
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
            connect_timeout_ms: 30_000,
            max_workspace_mb: Some(512),
            max_sessions: Some(50),
            max_checkpoints: Some(200),
            session_idle_timeout_secs: Some(3600),
            mount_allowlist: vec![],
            mount_overlay_paths: vec![
                "node_modules".to_string(),
                ".venv".to_string(),
                "venv".to_string(),
                "target".to_string(),
                "__pycache__".to_string(),
                ".git".to_string(),
            ],
            export_root: PathBuf::from("drun-export"),
            snapshots_dir: PathBuf::from("drun-snapshots"),
            snapshot_on_close: false,
            env_allowlist: vec![],
            bash_timeout_ms: 30_000,
            bash_command_denylist: vec![],
            bash_command_allowlist: vec![],
            web_port: Some(7274),
        }
    }
}

impl Config {
    pub fn domain_allowed(&self, host: &str) -> bool {
        self.domain_allowlist
            .iter()
            .any(|pattern| match pattern.as_str() {
                "*" => true,
                p if p.starts_with("*.") => host
                    .strip_suffix(&p[2..])
                    .and_then(|pre| pre.strip_suffix('.'))
                    .is_some(),
                p => p == host,
            })
    }

    pub fn load() -> Self {
        let path = std::env::var("DRUN_CONFIG").ok().map(PathBuf::from);
        Self::load_from(path.as_deref())
    }

    pub fn load_from(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(e) => {
                eprintln!("drun: failed to read config at {}: {e}", path.display());
                return Self::default();
            }
        };
        match toml::from_str::<Config>(&contents) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("drun: failed to parse config at {}: {e}", path.display());
                Self::default()
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConfigHandle {
    fixed: Config,
    path: Option<PathBuf>,
}

impl ConfigHandle {
    pub fn new(config: Config, path: Option<PathBuf>) -> Self {
        Self {
            fixed: config,
            path,
        }
    }

    pub fn from_env() -> Self {
        let path = std::env::var("DRUN_CONFIG").ok().map(PathBuf::from);
        let config = Config::load_from(path.as_deref());
        Self::new(config, path)
    }

    pub fn get(&self) -> Config {
        match &self.path {
            Some(path) => Config::load_from(Some(path)),
            None => self.fixed.clone(),
        }
    }
}

impl From<Config> for ConfigHandle {
    fn from(config: Config) -> Self {
        Self::new(config, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values_are_frozen() {
        let config = Config::default();
        assert_eq!(
            config.domain_allowlist,
            vec!["cdn.jsdelivr.net", "files.pythonhosted.org", "pypi.org"]
        );
        assert_eq!(config.fetch_timeout_ms, 60_000);
        assert_eq!(config.connect_timeout_ms, 30_000);
        assert_eq!(config.max_workspace_mb, Some(512));
        assert_eq!(config.max_sessions, Some(50));
        assert_eq!(config.max_checkpoints, Some(200));
        assert_eq!(config.session_idle_timeout_secs, Some(3600));
        assert_eq!(config.mount_allowlist, Vec::<PathBuf>::new());
        assert_eq!(
            config.mount_overlay_paths,
            vec![
                "node_modules",
                ".venv",
                "venv",
                "target",
                "__pycache__",
                ".git"
            ]
        );
        assert_eq!(config.export_root, PathBuf::from("drun-export"));
        assert_eq!(config.snapshots_dir, PathBuf::from("drun-snapshots"));
        assert!(!config.snapshot_on_close);
        assert_eq!(config.env_allowlist, Vec::<String>::new());
        assert_eq!(config.bash_timeout_ms, 30_000);
        assert_eq!(config.bash_command_denylist, Vec::<String>::new());
        assert_eq!(config.bash_command_allowlist, Vec::<String>::new());
        assert_eq!(config.web_port, Some(7274));
    }

    #[test]
    fn exact_match_is_allowed() {
        let config = Config {
            domain_allowlist: vec!["pypi.org".to_string()],
            ..Config::default()
        };
        assert!(config.domain_allowed("pypi.org"));
        assert!(!config.domain_allowed("evil.org"));
    }

    #[test]
    fn wildcard_star_allows_everything() {
        let config = Config {
            domain_allowlist: vec!["*".to_string()],
            ..Config::default()
        };
        assert!(config.domain_allowed("anything.example"));
    }

    #[test]
    fn subdomain_wildcard_matches_subdomains_only() {
        let config = Config {
            domain_allowlist: vec!["*.example.com".to_string()],
            ..Config::default()
        };
        assert!(config.domain_allowed("api.example.com"));
        assert!(!config.domain_allowed("example.com")); // bare domain, not a subdomain
        assert!(!config.domain_allowed("api.evil.com"));
    }

    #[test]
    fn subdomain_wildcard_matches_multiple_levels_deep() {
        let config = Config {
            domain_allowlist: vec!["*.example.com".to_string()],
            ..Config::default()
        };
        assert!(config.domain_allowed("a.b.example.com"));
    }

    #[test]
    fn subdomain_wildcard_rejects_lookalike_suffix() {
        let config = Config {
            domain_allowlist: vec!["*.example.com".to_string()],
            ..Config::default()
        };
        assert!(!config.domain_allowed("evilexample.com"));
    }

    #[test]
    fn empty_allowlist_denies_everything() {
        let config = Config {
            domain_allowlist: vec![],
            ..Config::default()
        };
        assert!(!config.domain_allowed("pypi.org"));
    }

    #[test]
    fn any_matching_pattern_in_a_mixed_list_allows() {
        let config = Config {
            domain_allowlist: vec!["pypi.org".to_string(), "*.example.com".to_string()],
            ..Config::default()
        };
        assert!(config.domain_allowed("pypi.org"));
        assert!(config.domain_allowed("api.example.com"));
        assert!(!config.domain_allowed("evil.org"));
    }

    #[test]
    fn default_matches_documented_values() {
        let config = Config::default();
        assert_eq!(
            config.domain_allowlist,
            vec!["cdn.jsdelivr.net", "files.pythonhosted.org", "pypi.org"]
        );
        assert_eq!(config.fetch_timeout_ms, 60_000);
        assert_eq!(config.connect_timeout_ms, 30_000);
        assert_eq!(config.bash_timeout_ms, 30_000);
        assert_eq!(config.max_workspace_mb, Some(512));
        assert_eq!(config.max_sessions, Some(50));
        assert_eq!(config.max_checkpoints, Some(200));
        assert_eq!(config.session_idle_timeout_secs, Some(3600));
        assert_eq!(config.web_port, Some(7274));
        assert!(config.mount_allowlist.is_empty());
        assert!(config.env_allowlist.is_empty());
        assert!(config.bash_command_denylist.is_empty());
        assert!(config.bash_command_allowlist.is_empty());
        assert!(!config.snapshot_on_close);
        assert_eq!(config.export_root, PathBuf::from("drun-export"));
        assert_eq!(config.snapshots_dir, PathBuf::from("drun-snapshots"));
        assert_eq!(
            config.mount_overlay_paths,
            vec![
                "node_modules",
                ".venv",
                "venv",
                "target",
                "__pycache__",
                ".git"
            ]
        );
    }

    #[test]
    fn config_handle_without_a_path_stays_fixed() {
        let handle = ConfigHandle::from(Config {
            bash_timeout_ms: 1234,
            ..Config::default()
        });
        assert_eq!(handle.get().bash_timeout_ms, 1234);
        assert_eq!(handle.get().bash_timeout_ms, 1234);
    }

    #[test]
    fn config_handle_with_a_path_reflects_edits_made_after_construction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "bash_timeout_ms = 1000\n").unwrap();

        let handle = ConfigHandle::new(Config::load_from(Some(&path)), Some(path.clone()));
        assert_eq!(handle.get().bash_timeout_ms, 1000);

        std::fs::write(&path, "bash_timeout_ms = 2000\n").unwrap();
        assert_eq!(handle.get().bash_timeout_ms, 2000);
    }

    fn load_from_toml(contents: &str) -> Config {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, contents).unwrap();
        Config::load_from(Some(&path))
    }

    #[test]
    fn load_from_none_returns_defaults() {
        assert_eq!(Config::load_from(None), Config::default());
    }

    #[test]
    fn load_from_missing_file_returns_defaults() {
        let config = Config::load_from(Some(Path::new("/nonexistent/drun-config-test.toml")));
        assert_eq!(config, Config::default());
    }

    #[test]
    fn load_from_malformed_toml_returns_defaults() {
        let config = load_from_toml("this is not valid toml {{{");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn load_from_valid_toml_overrides_only_the_fields_it_sets() {
        let config =
            load_from_toml("bash_timeout_ms = 5000\ndomain_allowlist = [\"custom.example.com\"]\n");
        assert_eq!(config.bash_timeout_ms, 5000);
        // Untouched fields still come from #[serde(default)] -> Config::default().
        assert_eq!(config.fetch_timeout_ms, Config::default().fetch_timeout_ms);
        // An explicit domain_allowlist replaces the default outright — it is
        // no longer merged with the built-in domains, so an operator can
        // actually restrict session_fetch below the defaults.
        assert_eq!(
            config.domain_allowlist,
            vec!["custom.example.com".to_string()]
        );
    }

    #[test]
    fn load_from_missing_domain_allowlist_key_falls_back_to_builtin_domains() {
        let config = load_from_toml("bash_timeout_ms = 5000\n");
        assert_eq!(config.domain_allowlist, Config::default().domain_allowlist);
    }

    #[test]
    fn load_from_explicit_empty_domain_allowlist_denies_everything() {
        let config = load_from_toml("domain_allowlist = []\n");
        assert!(config.domain_allowlist.is_empty());
        assert!(!config.domain_allowed("pypi.org"));
    }
}

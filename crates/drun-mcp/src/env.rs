//! This drun installation's runtime environment — where its config and
//! project registry live, and which port the MCP server binds. Shared across
//! `main`, `config_cmd`, and every `Bridge`; not owned by any one of them.

use std::path::PathBuf;

pub(crate) struct Env;

impl Env {
    pub(crate) const DEFAULT_MCP_PORT: u16 = 7273;

    pub(crate) fn mcp_port(&self) -> u16 {
        std::env::var("DRUN_MCP_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(Self::DEFAULT_MCP_PORT)
    }

    pub(crate) fn home_dir(&self) -> PathBuf {
        PathBuf::from(std::env::var("HOME").expect("HOME not set"))
    }

    pub(crate) fn drun_home(&self) -> PathBuf {
        self.home_dir().join(".drun")
    }
}

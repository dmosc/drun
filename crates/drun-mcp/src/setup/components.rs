//! Detects which pieces of the drun stack (Ollama, the `drun chat` CLI,
//! Tailscale, the daemon itself) are already installed/running, and builds
//! the shell command that installs each one — used by both the status
//! endpoint and the job-runner endpoint in `super`.

use serde::Serialize;
use std::net::{SocketAddr, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;

#[derive(Serialize)]
pub(crate) struct SetupStatus {
    pub(crate) daemon: DaemonStatus,
    pub(crate) ollama: BinaryStatus,
    pub(crate) chat_cli: BinaryStatus,
    pub(crate) tailscale: TailscaleStatus,
}

#[derive(Serialize)]
pub(crate) struct DaemonStatus {
    pub(crate) running: bool,
    pub(crate) web_port: Option<u16>,
    pub(crate) mcp_port: u16,
}

#[derive(Serialize)]
pub(crate) struct BinaryStatus {
    pub(crate) installed: bool,
}

#[derive(Serialize)]
pub(crate) struct TailscaleStatus {
    pub(crate) installed: bool,
    pub(crate) authenticated: bool,
}

pub(crate) fn current_status() -> SetupStatus {
    let config = drun_core::Config::load();
    SetupStatus {
        daemon: daemon_status(&config),
        ollama: BinaryStatus {
            installed: command_exists("ollama"),
        },
        chat_cli: BinaryStatus {
            installed: chat_cli_installed(),
        },
        tailscale: tailscale_status(),
    }
}

fn daemon_status(config: &drun_core::Config) -> DaemonStatus {
    let mcp_port = crate::Env.mcp_port();
    DaemonStatus {
        running: port_open(mcp_port),
        web_port: config.web_port.filter(|&p| p != 0),
        mcp_port,
    }
}

fn port_open(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// True if `bin --version` runs at all — matches the check `bridge::Bridge`
/// uses for provider CLIs (`cli_mcp_available`): a nonzero exit still proves
/// the binary is on `PATH`, only "not found" means it isn't.
pub(crate) fn command_exists(bin: &str) -> bool {
    match Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(_) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

fn chat_cli_installed() -> bool {
    ["pip3", "pip"].iter().any(|pip| {
        Command::new(pip)
            .args(["show", "drun-sandbox"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

fn tailscale_status() -> TailscaleStatus {
    let installed = command_exists("tailscale");
    if !installed {
        return TailscaleStatus {
            installed: false,
            authenticated: false,
        };
    }
    let authenticated = Command::new("tailscale")
        .args(["status", "--json"])
        .stdin(Stdio::null())
        .output()
        .ok()
        .and_then(|output| serde_json::from_slice::<serde_json::Value>(&output.stdout).ok())
        .and_then(|value| {
            value
                .get("BackendState")
                .and_then(|state| state.as_str().map(|s| s == "Running"))
        })
        .unwrap_or(false);
    TailscaleStatus {
        installed,
        authenticated,
    }
}

#[derive(Debug)]
pub(crate) enum JobLookupError {
    Unknown,
    /// No automated install path exists on this platform; the message
    /// points the user at the manual fallback.
    Unavailable(String),
}

/// Resolves a job id (as posted by the wizard UI) to the shell command it
/// should run right now. Kept as a flat match rather than a struct-per-job
/// table since there are only a handful of jobs and each needs different
/// inputs (`web_port` only matters for `tailscale_serve`).
pub(crate) fn command_for(job_id: &str, web_port: u16) -> Result<String, JobLookupError> {
    match job_id {
        "install_ollama" => install_ollama_command().ok_or_else(|| {
            JobLookupError::Unavailable(
                "No supported installer found (Homebrew not on PATH). Install Ollama manually: https://ollama.com/download".into(),
            )
        }),
        "pull_model" => Ok("ollama pull qwen3.6:latest".to_string()),
        "install_chat_cli" => install_chat_cli_command().ok_or_else(|| {
            JobLookupError::Unavailable(
                "pip not found. Install Python 3.9+ first: https://www.python.org/downloads/"
                    .into(),
            )
        }),
        "install_tailscale" => install_tailscale_command().ok_or_else(|| {
            JobLookupError::Unavailable(
                "No supported installer found (Homebrew not on PATH). Install Tailscale manually: https://tailscale.com/download".into(),
            )
        }),
        "tailscale_up" => Ok("tailscale up".to_string()),
        "tailscale_serve" => Ok(format!(
            "tailscale serve --bg --https=443 http://127.0.0.1:{web_port}"
        )),
        _ => Err(JobLookupError::Unknown),
    }
}

fn install_ollama_command() -> Option<String> {
    match std::env::consts::OS {
        "macos" if command_exists("brew") => Some("brew install ollama".to_string()),
        "linux" => Some("curl -fsSL https://ollama.com/install.sh | sh".to_string()),
        _ => None,
    }
}

fn install_chat_cli_command() -> Option<String> {
    if command_exists("pip3") {
        Some("pip3 install --user --upgrade 'drun-sandbox[chat]'".to_string())
    } else if command_exists("pip") {
        Some("pip install --user --upgrade 'drun-sandbox[chat]'".to_string())
    } else {
        None
    }
}

fn install_tailscale_command() -> Option<String> {
    match std::env::consts::OS {
        "macos" if command_exists("brew") => Some("brew install tailscale".to_string()),
        "linux" => Some("curl -fsSL https://tailscale.com/install.sh | sh".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_exists_is_false_for_a_binary_that_does_not_exist() {
        assert!(!command_exists(
            "definitely-not-a-real-binary-drun-setup-test"
        ));
    }

    #[test]
    fn command_exists_is_true_for_a_binary_known_to_be_on_path() {
        assert!(command_exists("sh"));
    }

    #[test]
    fn command_for_rejects_an_unknown_job_id() {
        assert!(matches!(
            command_for("not_a_real_job", 7274),
            Err(JobLookupError::Unknown)
        ));
    }

    #[test]
    fn command_for_tailscale_serve_embeds_the_given_web_port() {
        let command = command_for("tailscale_serve", 9999).unwrap();
        assert!(command.contains("9999"));
    }

    #[test]
    fn command_for_pull_model_and_tailscale_up_need_no_platform_detection() {
        assert_eq!(
            command_for("pull_model", 7274).unwrap(),
            "ollama pull qwen3.6:latest"
        );
        assert_eq!(command_for("tailscale_up", 7274).unwrap(), "tailscale up");
    }

    #[test]
    fn daemon_status_reports_the_configured_ports() {
        let config = drun_core::Config {
            web_port: Some(1234),
            ..Default::default()
        };
        let status = daemon_status(&config);
        assert_eq!(status.web_port, Some(1234));
        assert_eq!(status.mcp_port, crate::Env.mcp_port());
    }

    #[test]
    fn daemon_status_treats_a_zero_web_port_as_disabled() {
        let config = drun_core::Config {
            web_port: Some(0),
            ..Default::default()
        };
        assert_eq!(daemon_status(&config).web_port, None);
    }
}

//! Detection and shell-command lookup for the setup wizard's components.

use serde::Serialize;
use std::net::{SocketAddr, TcpStream};
use std::process::{Command, Stdio};
use std::time::Duration;

const OLLAMA_PORT: u16 = 11434;
const OLLAMA_MODEL: &str = "qwen3.6:latest";

#[derive(Serialize)]
pub(crate) struct SetupStatus {
    pub(crate) platform: &'static str,
    pub(crate) daemon: DaemonStatus,
    pub(crate) homebrew: BinaryStatus,
    pub(crate) ollama: OllamaStatus,
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
pub(crate) struct OllamaStatus {
    pub(crate) installed: bool,
    pub(crate) running: bool,
    pub(crate) model_pulled: bool,
}

#[derive(Serialize)]
pub(crate) struct TailscaleStatus {
    pub(crate) installed: bool,
    pub(crate) daemon_running: bool,
    pub(crate) authenticated: bool,
    pub(crate) sharing: bool,
}

pub(crate) fn current_status() -> SetupStatus {
    let config = drun_core::Config::load();
    SetupStatus {
        platform: std::env::consts::OS,
        daemon: daemon_status(&config),
        homebrew: BinaryStatus {
            installed: command_exists("brew"),
        },
        ollama: ollama_status(),
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

fn ollama_status() -> OllamaStatus {
    let installed = command_exists("ollama");
    let running = installed && port_open(OLLAMA_PORT);
    let model_pulled = running && output_of("ollama", &["list"]).contains(OLLAMA_MODEL);
    OllamaStatus {
        installed,
        running,
        model_pulled,
    }
}

fn chat_cli_installed() -> bool {
    ["pip3", "pip"]
        .iter()
        .any(|pip| succeeds(pip, &["show", "drun-sandbox"]))
}

fn tailscale_status() -> TailscaleStatus {
    let installed = command_exists("tailscale");
    if !installed {
        return TailscaleStatus {
            installed: false,
            daemon_running: false,
            authenticated: false,
            sharing: false,
        };
    }
    let status =
        serde_json::from_str::<serde_json::Value>(&output_of("tailscale", &["status", "--json"]))
            .ok();
    let daemon_running = status.is_some();
    let authenticated = status
        .as_ref()
        .and_then(|v| v.get("BackendState")?.as_str())
        .is_some_and(|s| s == "Running");
    let sharing = daemon_running
        && !output_of("tailscale", &["serve", "status"])
            .trim()
            .is_empty();
    TailscaleStatus {
        installed,
        daemon_running,
        authenticated,
        sharing,
    }
}

fn port_open(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// True if `bin --version` runs at all — a nonzero exit still proves the
/// binary is on `PATH`; only "not found" means it isn't.
pub(crate) fn command_exists(bin: &str) -> bool {
    match Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

fn succeeds(bin: &str, args: &[&str]) -> bool {
    Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn output_of(bin: &str, args: &[&str]) -> String {
    Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

#[derive(Debug)]
pub(crate) enum JobLookupError {
    Unknown,
    Unavailable(String),
}

type JobResult = Result<String, JobLookupError>;

/// Resolves a job id (as posted by the wizard UI) to the shell command it
/// should run right now. A flat match rather than a struct-per-job table:
/// there's a fixed, small set of jobs and each maps to one command builder.
pub(crate) fn command_for(job_id: &str, web_port: u16) -> JobResult {
    match job_id {
        "install_homebrew" => Ok(
            "NONINTERACTIVE=1 /bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\""
                .to_string(),
        ),

        "install_ollama" => brew_or_curl("ollama", "https://ollama.com/install.sh", "https://ollama.com/download"),
        "uninstall_ollama" => brew_uninstall("ollama"),
        "start_ollama" => Ok(brew_service("ollama", "start").unwrap_or_else(|| "nohup ollama serve > /dev/null 2>&1 &".to_string())),
        "stop_ollama" => Ok(brew_service("ollama", "stop").unwrap_or_else(|| "pkill -x ollama".to_string())),
        "pull_model" => Ok(format!("ollama pull {OLLAMA_MODEL}")),
        "remove_model" => Ok(format!("ollama rm {OLLAMA_MODEL}")),

        "install_chat_cli" => pip("install --user --upgrade", "'drun-sandbox[chat]'"),
        "uninstall_chat_cli" => pip("uninstall -y", "drun-sandbox"),

        "install_tailscale" => brew_or_curl("tailscale", "https://tailscale.com/install.sh", "https://tailscale.com/download"),
        "uninstall_tailscale" => brew_uninstall("tailscale"),
        "start_tailscale" => Ok(tailscaled(true)),
        "stop_tailscale" => Ok(tailscaled(false)),
        "tailscale_up" => Ok("tailscale up".to_string()),
        "tailscale_logout" => Ok("tailscale logout".to_string()),
        "tailscale_serve" => Ok(format!("tailscale serve --bg --https=443 http://127.0.0.1:{web_port}")),
        "tailscale_unserve" => Ok("tailscale serve --https=443 off".to_string()),

        _ => Err(JobLookupError::Unknown),
    }
}

fn brew_or_curl(formula: &str, curl_install_url: &str, manual_url: &str) -> JobResult {
    match std::env::consts::OS {
        "macos" if command_exists("brew") => Ok(format!("brew install {formula}")),
        "linux" => Ok(format!("curl -fsSL {curl_install_url} | sh")),
        _ => Err(JobLookupError::Unavailable(format!(
            "No supported installer found (Homebrew not on PATH). Install manually: {manual_url}"
        ))),
    }
}

fn brew_uninstall(formula: &str) -> JobResult {
    if command_exists("brew") {
        Ok(format!("brew uninstall {formula}"))
    } else {
        Err(JobLookupError::Unavailable(format!(
            "Homebrew not on PATH — remove {formula} the same way you installed it."
        )))
    }
}

fn brew_service(formula: &str, action: &str) -> Option<String> {
    command_exists("brew").then(|| format!("brew services {action} {formula}"))
}

fn pip(action: &str, package: &str) -> JobResult {
    ["pip3", "pip"]
        .into_iter()
        .find(|pip| command_exists(pip))
        .map(|pip| Ok(format!("{pip} {action} {package}")))
        .unwrap_or_else(|| {
            Err(JobLookupError::Unavailable(
                "pip not found. Install Python 3.9+ first: https://www.python.org/downloads/"
                    .to_string(),
            ))
        })
}

/// `tailscaled` manages a network interface, so unlike Ollama's server this
/// always needs root — expect it to fail under the wizard's no-stdin job
/// runner with a "password is required" error rather than actually run.
/// Still worth a button: the failure output is the exact command to paste
/// into a real terminal once.
fn tailscaled(start: bool) -> String {
    if std::env::consts::OS == "linux" {
        let action = if start {
            "enable --now"
        } else {
            "disable --now"
        };
        return format!("sudo systemctl {action} tailscaled");
    }
    match brew_service("tailscale", if start { "start" } else { "stop" }) {
        Some(command) => format!("sudo {command}"),
        None if start => "sudo tailscaled".to_string(),
        None => "sudo pkill tailscaled".to_string(),
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
        assert!(
            command_for("tailscale_serve", 9999)
                .unwrap()
                .contains("9999")
        );
    }

    #[test]
    fn command_for_jobs_needing_no_platform_detection() {
        assert_eq!(
            command_for("pull_model", 7274).unwrap(),
            "ollama pull qwen3.6:latest"
        );
        assert_eq!(
            command_for("remove_model", 7274).unwrap(),
            "ollama rm qwen3.6:latest"
        );
        assert_eq!(command_for("tailscale_up", 7274).unwrap(), "tailscale up");
        assert_eq!(
            command_for("tailscale_logout", 7274).unwrap(),
            "tailscale logout"
        );
        assert_eq!(
            command_for("tailscale_unserve", 7274).unwrap(),
            "tailscale serve --https=443 off"
        );
    }

    #[test]
    fn command_for_install_homebrew_is_always_available() {
        assert!(command_for("install_homebrew", 7274).is_ok());
    }

    #[test]
    fn command_for_start_and_stop_ollama_are_always_available() {
        assert!(command_for("start_ollama", 7274).is_ok());
        assert!(command_for("stop_ollama", 7274).is_ok());
    }

    #[test]
    fn command_for_start_and_stop_tailscale_always_need_sudo() {
        assert!(
            command_for("start_tailscale", 7274)
                .unwrap()
                .starts_with("sudo ")
        );
        assert!(
            command_for("stop_tailscale", 7274)
                .unwrap()
                .starts_with("sudo ")
        );
    }

    #[test]
    fn command_for_uninstall_jobs_require_homebrew() {
        let result = command_for("uninstall_ollama", 7274);
        assert_eq!(result.is_ok(), command_exists("brew"));
    }

    #[test]
    fn ollama_status_reports_nothing_running_when_not_installed() {
        if !command_exists("ollama") {
            let status = ollama_status();
            assert!(!status.installed && !status.running && !status.model_pulled);
        }
    }

    #[test]
    fn tailscale_status_reports_nothing_running_when_not_installed() {
        if !command_exists("tailscale") {
            let status = tailscale_status();
            assert!(
                !status.installed
                    && !status.daemon_running
                    && !status.authenticated
                    && !status.sharing
            );
        }
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

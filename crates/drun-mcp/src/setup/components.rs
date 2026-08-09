//! Detection and shell-command lookup for the setup wizard's components.

use serde::Serialize;
use std::collections::BTreeMap;
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
    pub(crate) commands: BTreeMap<&'static str, String>,
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
    let web_port = config.web_port.unwrap_or(7274);
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
        commands: commands(web_port),
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
    let sharing =
        daemon_running && is_serving(&output_of("tailscale", &["serve", "status", "--json"]));
    TailscaleStatus {
        installed,
        daemon_running,
        authenticated,
        sharing,
    }
}

fn is_serving(serve_status_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(serve_status_json)
        .is_ok_and(|v| v.as_object().is_some_and(|obj| !obj.is_empty()))
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

/// Every command the wizard shows, keyed by the id the UI's `CARDS` table
/// references. Display text only — a manual instruction (`# ...`) reads as
/// a harmless shell comment if copied as-is, so no id needs special-casing
/// on the frontend just because it isn't a real command on this platform.
fn commands(web_port: u16) -> BTreeMap<&'static str, String> {
    BTreeMap::from([
        ("install_homebrew", "NONINTERACTIVE=1 /bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"".to_string()),
        ("install_ollama", brew_or_curl("ollama", "https://ollama.com/install.sh", "https://ollama.com/download")),
        ("uninstall_ollama", brew_uninstall("ollama")),
        ("start_ollama", brew_service("ollama", "start").unwrap_or_else(|| "nohup ollama serve > /dev/null 2>&1 &".to_string())),
        ("stop_ollama", brew_service("ollama", "stop").unwrap_or_else(|| "pkill -x ollama".to_string())),
        ("pull_model", format!("ollama pull {OLLAMA_MODEL}")),
        ("remove_model", format!("ollama rm {OLLAMA_MODEL}")),
        ("install_chat_cli", pip("install --user --upgrade", "'drun-sandbox[chat]'")),
        ("uninstall_chat_cli", pip("uninstall -y", "drun-sandbox")),
        ("install_tailscale", brew_or_curl("tailscale", "https://tailscale.com/install.sh", "https://tailscale.com/download")),
        ("uninstall_tailscale", brew_uninstall("tailscale")),
        ("start_tailscale", tailscaled(true)),
        ("stop_tailscale", tailscaled(false)),
        ("tailscale_up", "tailscale up".to_string()),
        ("tailscale_logout", "tailscale logout".to_string()),
        ("tailscale_serve", format!("tailscale serve --bg --https=443 http://127.0.0.1:{web_port}")),
        ("tailscale_unserve", "tailscale serve --https=443 off".to_string()),
    ])
}

fn brew_or_curl(formula: &str, curl_install_url: &str, manual_url: &str) -> String {
    match std::env::consts::OS {
        "macos" if command_exists("brew") => format!("brew install {formula}"),
        "linux" => format!("curl -fsSL {curl_install_url} | sh"),
        _ => format!("# Homebrew not found — install {formula} manually: {manual_url}"),
    }
}

fn brew_uninstall(formula: &str) -> String {
    if command_exists("brew") {
        format!("brew uninstall {formula}")
    } else {
        format!("# Homebrew not found — remove {formula} the same way you installed it")
    }
}

fn brew_service(formula: &str, action: &str) -> Option<String> {
    command_exists("brew").then(|| format!("brew services {action} {formula}"))
}

fn pip(action: &str, package: &str) -> String {
    match ["pip3", "pip"].into_iter().find(|pip| command_exists(pip)) {
        Some(pip) => format!("{pip} {action} {package}"),
        None => "# pip not found — install Python 3.9+ first: https://www.python.org/downloads/"
            .to_string(),
    }
}

/// `tailscaled` manages a network interface, so unlike Ollama's server this
/// always needs root — shown as `sudo ...` so the copied command prompts
/// for a password in the user's own terminal.
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

/// Best-effort — if no terminal is found, the user just copies commands
/// into whichever one they already have open.
pub(crate) fn open_terminal() {
    if let Some((bin, args)) = terminal_launcher() {
        let _ = Command::new(bin).args(args).spawn();
    }
}

fn terminal_launcher() -> Option<(&'static str, &'static [&'static str])> {
    if std::env::consts::OS == "macos" {
        return Some(("open", &["-a", "Terminal"]));
    }
    ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"]
        .into_iter()
        .find(|t| command_exists(t))
        .map(|t| (t, &[][..]))
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
    fn commands_covers_every_id_the_wizard_ui_references() {
        let ids = commands(7274);
        for id in [
            "install_homebrew",
            "install_ollama",
            "uninstall_ollama",
            "start_ollama",
            "stop_ollama",
            "pull_model",
            "remove_model",
            "install_chat_cli",
            "uninstall_chat_cli",
            "install_tailscale",
            "uninstall_tailscale",
            "start_tailscale",
            "stop_tailscale",
            "tailscale_up",
            "tailscale_logout",
            "tailscale_serve",
            "tailscale_unserve",
        ] {
            assert!(ids.contains_key(id), "missing command for '{id}'");
        }
    }

    #[test]
    fn commands_needing_no_platform_detection_are_fixed_strings() {
        let ids = commands(7274);
        assert_eq!(ids["pull_model"], "ollama pull qwen3.6:latest");
        assert_eq!(ids["remove_model"], "ollama rm qwen3.6:latest");
        assert_eq!(ids["tailscale_up"], "tailscale up");
        assert_eq!(ids["tailscale_logout"], "tailscale logout");
        assert_eq!(ids["tailscale_unserve"], "tailscale serve --https=443 off");
    }

    #[test]
    fn tailscale_serve_command_embeds_the_given_web_port() {
        assert!(commands(9999)["tailscale_serve"].contains("9999"));
    }

    #[test]
    fn start_and_stop_tailscale_always_need_sudo() {
        let ids = commands(7274);
        assert!(ids["start_tailscale"].starts_with("sudo "));
        assert!(ids["stop_tailscale"].starts_with("sudo "));
    }

    #[test]
    fn uninstall_commands_fall_back_to_a_comment_without_homebrew() {
        let uninstall = &commands(7274)["uninstall_ollama"];
        if command_exists("brew") {
            assert!(uninstall.starts_with("brew uninstall"));
        } else {
            assert!(uninstall.starts_with('#'));
        }
    }

    #[test]
    fn terminal_launcher_uses_the_open_command_on_macos() {
        if std::env::consts::OS == "macos" {
            assert_eq!(terminal_launcher(), Some(("open", &["-a", "Terminal"][..])));
        }
    }

    #[test]
    fn is_serving_is_false_for_an_empty_config() {
        assert!(!is_serving("{}"));
    }

    #[test]
    fn is_serving_is_false_for_invalid_json() {
        assert!(!is_serving(""));
    }

    #[test]
    fn is_serving_is_true_once_something_is_configured() {
        assert!(is_serving(r#"{"TCP":{"443":{"HTTPS":true}}}"#));
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

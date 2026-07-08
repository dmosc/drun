use std::{net::TcpStream, path::PathBuf, process::Command, time::Duration};

const MCP_PORT: u16 = 7273;

/// Prints a summary of daemon health and current project state.
pub fn run() {
    let version = env!("CARGO_PKG_VERSION");
    let daemon_up = daemon_is_running();
    let mcp_up = port_is_reachable();
    let project_dir = initialized_project_dir();
    let config_path = crate::init::drun_home().join("config.toml");

    println!("drun v{version}");
    println!();

    if daemon_up {
        println!("daemon:   running");
    } else {
        println!("daemon:   stopped    (run: drun daemon start)");
    }

    if mcp_up {
        println!("mcp:      http://127.0.0.1:{MCP_PORT}/sse  [reachable]");
    } else {
        println!("mcp:      http://127.0.0.1:{MCP_PORT}/sse  [unreachable]");
    }

    if config_path.exists() {
        println!("config:   {}", config_path.display());
    } else {
        println!(
            "config:   {} [missing — run the drun installer]",
            config_path.display()
        );
    }

    println!();

    match project_dir {
        Some(dir) => println!("project:  initialized  ({})", dir.display()),
        None => println!("project:  not initialized  (run: drun init)"),
    }
}

/// Exposed for `drun daemon status` to reuse.
pub(crate) fn print_daemon_status() {
    if daemon_is_running() {
        eprintln!("drun: daemon is running");
    } else {
        eprintln!("drun: daemon is not running  (run: drun daemon start)");
    }
}

fn daemon_is_running() -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    match std::env::consts::OS {
        "macos" => {
            let plist =
                format!("{home}/Library/LaunchAgents/com.drun.mcp-server.plist");
            if !std::path::Path::new(&plist).exists() {
                return false;
            }
            Command::new("launchctl")
                .args(["list", "com.drun.mcp-server"])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        }
        "linux" => Command::new("systemctl")
            .args(["--user", "is-active", "--quiet", "drun-mcp.service"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        _ => false,
    }
}

/// Checks whether the MCP port is accepting connections.
fn port_is_reachable() -> bool {
    let addr = format!("127.0.0.1:{MCP_PORT}");
    TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(500)).is_ok()
}

/// Returns the current working directory if it has been initialized for drun.
fn initialized_project_dir() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let settings = cwd.join(".claude/settings.json");
    if !settings.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&settings).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;

    let allow = value.get("permissions")?.get("allow")?.as_array()?;
    if allow.iter().any(|v| v.as_str() == Some("mcp__drun__*")) {
        Some(cwd)
    } else {
        None
    }
}

use std::{path::Path, process::Command};

/// Platform-agnostic daemon lifecycle commands.
pub fn run(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("start") => start(),
        Some("stop") => stop(),
        Some("restart") => crate::config_cmd::restart_daemon(),
        Some("status") => crate::status::print_daemon_status(),
        Some("logs") => logs(),
        _ => {
            eprintln!("usage: drun daemon <start|stop|restart|status|logs>");
            std::process::exit(1);
        }
    }
}

fn start() {
    let home = std::env::var("HOME").unwrap_or_default();
    match std::env::consts::OS {
        "macos" => {
            let plist =
                format!("{home}/Library/LaunchAgents/com.drun.mcp-server.plist");
            if !Path::new(&plist).exists() {
                eprintln!("drun: no launchd agent found — run the drun installer first");
                std::process::exit(1);
            }
            match Command::new("launchctl")
                .args(["load", "-w", &plist])
                .status()
            {
                Ok(s) if s.success() => eprintln!("drun: daemon started"),
                _ => eprintln!("drun: failed to start daemon"),
            }
        }
        "linux" => {
            match Command::new("systemctl")
                .args(["--user", "start", "drun-mcp.service"])
                .status()
            {
                Ok(s) if s.success() => eprintln!("drun: daemon started"),
                _ => eprintln!("drun: failed to start daemon"),
            }
        }
        _ => {
            eprintln!("drun: unsupported platform");
            std::process::exit(1);
        }
    }
}

fn stop() {
    let home = std::env::var("HOME").unwrap_or_default();
    match std::env::consts::OS {
        "macos" => {
            let plist =
                format!("{home}/Library/LaunchAgents/com.drun.mcp-server.plist");
            match Command::new("launchctl").args(["unload", &plist]).status() {
                Ok(s) if s.success() => eprintln!("drun: daemon stopped"),
                _ => eprintln!("drun: failed to stop daemon (may not be running)"),
            }
        }
        "linux" => {
            match Command::new("systemctl")
                .args(["--user", "stop", "drun-mcp.service"])
                .status()
            {
                Ok(s) if s.success() => eprintln!("drun: daemon stopped"),
                _ => eprintln!("drun: failed to stop daemon (may not be running)"),
            }
        }
        _ => {
            eprintln!("drun: unsupported platform");
            std::process::exit(1);
        }
    }
}

fn logs() {
    // Check both names to handle installations before this binary rename
    let drun_home = crate::init::drun_home();
    let log_file = ["drun.log", "drun-mcp.log"]
        .iter()
        .map(|name| drun_home.join(name))
        .find(|p| p.exists());

    let Some(log_path) = log_file else {
        eprintln!("drun: no log file found in {}", drun_home.display());
        std::process::exit(1);
    };

    eprintln!("drun: tailing {}  (Ctrl-C to stop)", log_path.display());
    let _ = Command::new("tail")
        .args(["-f", log_path.to_str().unwrap()])
        .status();
}

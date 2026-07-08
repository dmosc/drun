use std::{path::Path, process::Command};

/// Full global teardown. Prompts for confirmation, then:
///   - stops and removes the launchd/systemd daemon
///   - deregisters drun from Claude Code
///   - removes ~/.drun/ (config, registry, logs)
///   - removes the drun binary itself
///
/// Per-project `.claude/settings.json` and `CLAUDE.md` files are NOT touched.
pub fn run() {
    print_plan();

    eprint!("Continue? [y/N] ");
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .expect("cannot read input");

    if !input.trim().eq_ignore_ascii_case("y") {
        eprintln!("drun: uninstall cancelled");
        return;
    }

    // Collect the binary path before we remove anything, since ~/.drun may
    // hold state we read during teardown
    let bin_path = current_binary_path();

    stop_daemon();
    deregister_mcp();
    remove_drun_home();
    // Binary is removed last — on Unix a running process can delete its own
    // on-disk inode; the in-memory image keeps running until we exit
    remove_binary(&bin_path);

    eprintln!("drun: uninstall complete");
    eprintln!(
        "      Per-project .claude/settings.json and CLAUDE.md files were left intact.\n\
         Run `drun deinit` in each project to clean those up."
    );
}

fn print_plan() {
    eprintln!(
        "This will remove:\n\
         \n\
         \x20 • the drun daemon (launchd/systemd service and plist/unit file)\n\
         \x20 • the drun binary\n\
         \x20 • drun's Claude Code MCP registration\n\
         \x20 • ~/.drun/  (config, project registry, logs)\n\
         \n\
         Not touched: per-project .claude/settings.json and CLAUDE.md files.\n"
    );
}

fn current_binary_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "/usr/local/bin/drun".to_string())
}

fn stop_daemon() {
    let home = std::env::var("HOME").unwrap_or_default();
    match std::env::consts::OS {
        "macos" => {
            let plist =
                format!("{home}/Library/LaunchAgents/com.drun.mcp-server.plist");
            if Path::new(&plist).exists() {
                let _ = Command::new("launchctl").args(["unload", &plist]).status();
                std::fs::remove_file(&plist).ok();
                eprintln!("drun: removed launchd agent");
            }
        }
        "linux" => {
            let service =
                format!("{home}/.config/systemd/user/drun-mcp.service");
            if Path::new(&service).exists() {
                let _ = Command::new("systemctl")
                    .args(["--user", "disable", "--now", "drun-mcp.service"])
                    .status();
                std::fs::remove_file(&service).ok();
                let _ = Command::new("systemctl")
                    .args(["--user", "daemon-reload"])
                    .status();
                eprintln!("drun: removed systemd user service");
            }
        }
        _ => {}
    }
    // Kill any stray processes regardless of platform
    let _ = Command::new("pkill").args(["-f", "drun"]).status();
}

fn deregister_mcp() {
    if Command::new("claude")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        let _ = Command::new("claude")
            .args(["mcp", "remove", "--scope", "user", "drun"])
            .status();
        eprintln!("drun: removed MCP registration from Claude Code");
    }
}

fn remove_drun_home() {
    let drun_home = crate::init::drun_home();
    if drun_home.exists() {
        std::fs::remove_dir_all(&drun_home).ok();
        eprintln!("drun: removed {}", drun_home.display());
    }
}

fn remove_binary(bin: &str) {
    // Try direct removal first; if the binary is owned by root, use sudo
    if std::fs::remove_file(bin).is_err() {
        let _ = Command::new("sudo").args(["rm", "-f", bin]).status();
    }
    eprintln!("drun: removed binary {bin}");
}

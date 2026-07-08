mod config_cmd;
mod daemon_cmd;
mod deinit;
mod errors;
mod handler;
mod init;
mod projects;
mod reaper;
mod response;
mod server;
mod state;
mod status;
mod tools;
mod uninstall;
mod update_cmd;
mod web;

use drun_core::Config;
use handler::DrunHandler;
use rust_mcp_sdk::{
    ToMcpServerHandler,
    error::SdkResult,
    mcp_server::{HyperServerOptions, hyper_server},
    schema::{
        Implementation, InitializeResult, ProtocolVersion, ServerCapabilities,
        ServerCapabilitiesTools,
    },
};

const MCP_PORT: u16 = 7273;

#[tokio::main]
async fn main() -> SdkResult<()> {
    match std::env::args().nth(1).as_deref() {
        Some("init") => {
            let target = std::env::args().nth(2).map(std::path::PathBuf::from);
            init::run(target);
            return Ok(());
        }
        Some("deinit") => {
            let target = std::env::args().nth(2).map(std::path::PathBuf::from);
            deinit::run(target);
            return Ok(());
        }
        Some("update") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            update_cmd::run(&args);
            return Ok(());
        }
        Some("uninstall") => {
            uninstall::run();
            return Ok(());
        }
        Some("status") => {
            status::run();
            return Ok(());
        }
        Some("projects") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            projects::run(&args);
            return Ok(());
        }
        Some("daemon") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            daemon_cmd::run(&args);
            return Ok(());
        }
        Some("config") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            config_cmd::run(&args);
            return Ok(());
        }
        Some("--help" | "-h") => {
            print_usage();
            return Ok(());
        }
        _ => {}
    }

    // No recognized subcommand — start the MCP daemon
    let handler = DrunHandler::new(Config::load());
    handler.start_idle_reaper();

    if let Some(web_port) = handler.config.web_port.filter(|&p| p != 0) {
        let web_sessions = std::sync::Arc::clone(&handler.sessions);
        tokio::spawn(web::WebServer::new(web_sessions, web_port).serve());
    }

    eprintln!("drun: MCP → http://127.0.0.1:{MCP_PORT}/mcp (streamable HTTP)");
    eprintln!("drun: MCP → http://127.0.0.1:{MCP_PORT}/sse (SSE)");

    hyper_server::create_server(
        build_server_details(),
        handler.to_mcp_server_handler(),
        HyperServerOptions {
            host: "127.0.0.1".into(),
            port: MCP_PORT,
            ..Default::default()
        },
    )
    .start()
    .await
}

fn print_usage() {
    let v = env!("CARGO_PKG_VERSION");
    eprintln!("drun v{v} — sandboxed execution for agentic loops");
    eprintln!();
    eprintln!("USAGE");
    eprintln!("  drun <command> [args]");
    eprintln!();
    eprintln!("PROJECT");
    eprintln!("  drun init [dir]                  set up drun for a project directory (default: cwd)");
    eprintln!("  drun deinit [dir]                remove drun setup from a project directory");
    eprintln!();
    eprintln!("GLOBAL");
    eprintln!("  drun update                      update binary, preserve settings, re-init projects");
    eprintln!("  drun update --skip-reinit         update binary only, skip project re-initialization");
    eprintln!("  drun update --version <tag>       update to a specific release (e.g. v0.4.0)");
    eprintln!("  drun uninstall                   remove drun entirely (prompts for confirmation)");
    eprintln!();
    eprintln!("OBSERVABILITY");
    eprintln!("  drun status                      show daemon health and current project state");
    eprintln!("  drun projects                    list all registered project directories");
    eprintln!("  drun projects --clean             remove stale entries from the project registry");
    eprintln!();
    eprintln!("DAEMON");
    eprintln!("  drun daemon start                start the background daemon");
    eprintln!("  drun daemon stop                 stop the background daemon");
    eprintln!("  drun daemon restart              restart the background daemon");
    eprintln!("  drun daemon status               show whether the daemon is running");
    eprintln!("  drun daemon logs                 tail the daemon log file");
    eprintln!();
    eprintln!("CONFIG");
    eprintln!("  drun config list                 show current domain and path allowlists");
    eprintln!("  drun config add-domain <name>    allow a domain for session_fetch");
    eprintln!("  drun config remove-domain <name>  revoke a domain");
    eprintln!("  drun config add-path <path>      allow a host path for session_mount");
    eprintln!("  drun config remove-path <path>   revoke a path");
    eprintln!();
    eprintln!("Running `drun` with no arguments starts the MCP server (managed automatically");
    eprintln!("by launchd/systemd — you should not need to invoke this directly).");
}

fn build_server_details() -> InitializeResult {
    InitializeResult {
        server_info: Implementation {
            name: "drun".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            title: Some("drun".into()),
            description: Some("Sandboxed code execution for agentic loops".into()),
            icons: vec![],
            website_url: None,
        },
        capabilities: ServerCapabilities {
            tools: Some(ServerCapabilitiesTools { list_changed: None }),
            ..Default::default()
        },
        protocol_version: ProtocolVersion::V2025_11_25.into(),
        instructions: Some("Go to https://drun.dev to view docs.".into()),
        meta: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_server_details_reports_the_crate_version() {
        let details = build_server_details();
        assert_eq!(details.server_info.name, "drun");
        assert_eq!(details.server_info.version, env!("CARGO_PKG_VERSION"));
    }
}
